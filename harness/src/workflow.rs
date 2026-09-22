use crate::{
    agents::{Agent, Completion, Outcome},
    archive,
    config::Config,
    process,
    protocol::{
        self, AgentIdentity, ArtifactStatus, EnvironmentIdentity, Export, Run, RunStore, Task,
        atomic_json,
    },
    replay,
    sandbox::{Guest, Role, Sandbox},
};
use anyhow::{Result, ensure};
use chrono::Utc;
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

/// Explicit lifecycle order, shared by the real controller and unit-test backend.
/// A prerequisite failure is NOT an attempt: allocate only after preflight passes.
pub struct Attempt<'a> {
    pub agent: Agent,
    pub model: &'a str,
    pub task_version: &'a str,
}

pub fn run<B: Sandbox>(
    backend: &B,
    config: &Config,
    repo: &Path,
    state: &Path,
    attempt: Attempt<'_>,
    abort: &AtomicBool,
) -> Result<String> {
    let Attempt {
        agent,
        model,
        task_version,
    } = attempt;
    crate::config::validate_model(model).map_err(anyhow::Error::msg)?;
    let task = Task::load(repo, task_version)?;
    ensure!(
        task.contract.environment == config.environment,
        "task/environment mismatch"
    );
    let store = RunStore::open(repo)?;
    store.check_frozen(&task)?;
    backend.preflight(Role::Generation(agent))?;
    ensure!(
        !abort.load(Ordering::SeqCst),
        "aborted before attempt allocation"
    );
    let now = Utc::now();
    let id = protocol::run_id(agent, model, now);
    let mut inputs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    inputs.insert(
        "settings.json".into(),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1, "backend": config.backend, "environment": config.environment,
            "limits": config.limits, "agent": agent.settings()
        }))?,
    );
    let environment_lock = repo
        .join("environments")
        .join(&config.environment)
        .join("environment.lock.json");
    if environment_lock.exists() {
        inputs.insert(
            "environment.lock.json".into(),
            archive::read_regular(&environment_lock, 1024 * 1024)?,
        );
    }
    let mut run = Run {
        schema_version: protocol::SCHEMA,
        protocol_version: protocol::PROTOCOL.into(),
        run_id: id.clone(),
        task_version: task_version.into(),
        prompt_sha256: task.prompt_sha256.clone(),
        task_sha256: task.contract_sha256.clone(),
        input_sha256: inputs
            .iter()
            .map(|(name, bytes)| (name.clone(), archive::hash(bytes)))
            .collect(),
        harness: protocol::harness_identity(repo),
        agent: AgentIdentity {
            name: agent,
            cli_version: None,
            requested_model: model.into(),
            reported_model: None,
            invocation: agent.invocation(model),
            settings: agent.settings(),
        },
        environment: EnvironmentIdentity {
            backend: "sbx".into(),
            backend_version: None,
            environment: config.environment.clone(),
            image_digest: None,
            rust: None,
            architecture: None,
            effective_limits: None,
            network_policy: None,
            isolation_verified: false,
        },
        requested_limits: config.limits.clone(),
        allocated_at: now,
        started_at: None,
        ended_at: None,
        elapsed_seconds: None,
        outcome: Outcome::default(),
        export: Export::default(),
        cleanup: ArtifactStatus::Pending,
        replay_preparation: ArtifactStatus::Pending,
    };
    let dir = store.allocate(
        &run,
        &task,
        &archive::read_regular(&repo.join("templates/review.md"), 65536)?,
        &inputs,
    )?;
    drop(store); // Task freeze is now represented by the atomic snapshot.
    atomic_json(&dir.join("replay.json"), &replay::Replay::default())?;
    let raw = state.join("raw").join(&id);
    // Host-only recovery journal is written before provisioning starts.
    let result = (|| -> Result<()> {
        fs::create_dir_all(&raw)?;
        archive::private_dir(&raw)?;
        eprintln!("Allocated {id}; creating a fresh guest.");
        let name = format!("bench-{}", uuid::Uuid::new_v4().simple());
        atomic_json(
            &raw.join("guest.json"),
            &json!({"guest": name, "state": "provisioning"}),
        )?;
        let guest = Guest::named(backend, Role::Generation(agent), name)?;
        atomic_json(
            &raw.join("guest.json"),
            &json!({"guest": guest.name, "state": "allocated"}),
        )?;
        run.environment = backend.verify(&guest.name, Role::Generation(agent))?;
        ensure!(
            run.environment.isolation_verified
                && run.environment.environment == config.environment
                && run.environment.effective_limits.is_some()
                && run.environment.network_policy.is_some(),
            "backend supplied incomplete isolation evidence"
        );
        let version = process::control_logged_abort(
            backend.exec(&guest.name, &[agent.to_string(), "--version".into()], false)?,
            &raw.join("setup"),
            abort,
        )?;
        let version = std::str::from_utf8(&version)?.trim();
        ensure!(
            version == agent.version_banner(),
            "executed agent version differs from environment pin"
        );
        run.agent.cli_version = Some(agent.pinned_version().into());
        // Git is initialized inside a new /workspace; never transfer parent .git.
        process::control_logged_abort(
            backend.exec(
                &guest.name,
                &[
                    "git".into(),
                    "-c".into(),
                    "init.templateDir=".into(),
                    "init".into(),
                    "--quiet".into(),
                    ".".into(),
                ],
                false,
            )?,
            &raw.join("setup"),
            abort,
        )?;
        ensure!(
            !abort.load(Ordering::SeqCst),
            "aborted before prompt delivery"
        );
        run.started_at = Some(Utc::now());
        atomic_json(&dir.join("run.json"), &run)?;
        let clock = Instant::now();
        eprintln!("Delivering the frozen prompt once; waiting for natural completion.");
        let request = process::agent_request(
            backend.exec(&guest.name, &run.agent.invocation, false)?,
            agent,
            &task.prompt,
            &raw,
            config.limits.log_bytes,
            abort,
        );
        run.elapsed_seconds = Some(clock.elapsed().as_secs_f64());
        run.ended_at = Some(Utc::now());
        match request {
            Ok((outcome, reported_model)) => {
                run.outcome = outcome;
                run.agent.reported_model = reported_model;
            }
            Err(_) => {
                run.outcome.completion = Completion::InfrastructureFailure;
                run.outcome.stop_reason = "agent_transport_failed".into();
            }
        }
        // Stop all guest processes BEFORE any snapshot/export, including after abort.
        backend.stop(&guest.name)?;
        atomic_json(
            &raw.join("guest.json"),
            &json!({"guest": guest.name, "state": "stopped"}),
        )?;
        let export = backend.export(
            &guest.name,
            "workspace",
            &dir.join("solution"),
            &raw,
            config.limits.export_bytes,
            config.limits.export_files,
        );
        match export {
            Ok(()) => {
                let files = archive::inventory(
                    &dir.join("solution"),
                    config.limits.export_bytes,
                    config.limits.export_files,
                )?;
                run.export = Export {
                    status: ArtifactStatus::Complete,
                    reason: None,
                    tree_sha256: Some(archive::tree_hash(&files)?),
                    files,
                };
                archive::make_readonly(&dir.join("solution"))?;
            }
            Err(_) => {
                run.export.status = ArtifactStatus::Failed;
                run.export.reason =
                    Some("unsafe_or_failed_source_export; see private diagnostics".into());
            }
        }
        guest.destroy()?;
        run.cleanup = ArtifactStatus::Complete;
        atomic_json(&raw.join("guest.json"), &json!({"state": "destroyed"}))?;
        Ok(())
    })();
    if result.is_err() {
        if run.outcome.completion == Completion::NotStarted {
            run.outcome.completion = if abort.load(Ordering::SeqCst) {
                Completion::UserAbort
            } else {
                Completion::InfrastructureFailure
            };
            run.outcome.stop_reason = "guest_setup_or_verification_failed_before_delivery".into();
        } else if run.outcome.completion == Completion::Normal {
            // Agent completion remains normal: post-attempt failure is separate.
            run.export.reason = Some("post_completion_infrastructure_failure".into());
        }
        if run.export.status == ArtifactStatus::Pending {
            run.export.status = ArtifactStatus::Failed;
        }
        // Drop attempted destruction, but cannot assert success without observing it.
        if run.cleanup != ArtifactStatus::Complete {
            run.cleanup = ArtifactStatus::Unavailable;
        }
    }
    run.ended_at.get_or_insert_with(Utc::now);
    atomic_json(&dir.join("run.json"), &run)?;
    if run.export.status == ArtifactStatus::Complete
        && !abort.load(Ordering::SeqCst)
        && result.is_ok()
    {
        eprintln!("Source preserved; preparing locked dependencies in a separate guest.");
        match replay::prepare_abort(backend, config, &dir, &run, state, abort) {
            Ok(replay) => run.replay_preparation = replay.preparation,
            Err(_) => run.replay_preparation = ArtifactStatus::Failed,
        }
    } else {
        run.replay_preparation = ArtifactStatus::Unavailable;
        atomic_json(
            &dir.join("replay.json"),
            &replay::Replay {
                preparation: ArtifactStatus::Unavailable,
                reason: Some("source_unavailable_or_attempt_aborted".into()),
                ..replay::Replay::default()
            },
        )?;
    }
    atomic_json(&dir.join("run.json"), &run)?;
    eprintln!(
        "Archived {id}: {:?}; export {:?}; replay {:?}.",
        run.outcome.completion, run.export.status, run.replay_preparation
    );
    result?;
    Ok(id)
}
