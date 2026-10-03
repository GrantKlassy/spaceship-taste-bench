//! Simulated control flow only: these tests make no claim about real VM isolation.
use anyhow::{Result, bail};
use bench::{
    agents::{Agent, Completion},
    archive,
    config::Config,
    protocol::{ArtifactStatus, EnvironmentIdentity, Run, RunStore, Task},
    sandbox::{Guest, Role, Sandbox},
    workflow,
};
use serde_json::json;
use std::{cell::RefCell, fs, path::Path, process::Command, sync::atomic::AtomicBool};

struct Fake {
    calls: RefCell<Vec<String>>,
    fail: &'static str,
    environment: &'static str,
}
impl Fake {
    fn note(&self, action: &str) -> Result<()> {
        self.calls.borrow_mut().push(action.into());
        if self.fail == action {
            bail!("injected {action}");
        }
        Ok(())
    }
}
impl Sandbox for Fake {
    fn preflight(&self, _: Role) -> Result<()> {
        self.note("preflight")
    }
    fn create(&self, _: &str, _: Role) -> Result<()> {
        self.note("create")
    }
    fn verify(&self, _: &str, _: Role) -> Result<EnvironmentIdentity> {
        self.note("verify")?;
        Ok(EnvironmentIdentity {
            backend: "unit-test-simulation".into(),
            backend_version: Some("fixture".into()),
            environment: self.environment.into(),
            image_digest: Some(format!("sha256:{}", "a".repeat(64))),
            rust: Some("1.97.0".into()),
            architecture: Some("x86_64".into()),
            effective_limits: Some(legacy_config().limits),
            network_policy: Some(json!({"fixture": true})),
            isolation_verified: self.fail != "uncertified",
        })
    }
    fn exec(&self, _: &str, args: &[String], _: bool) -> Result<Command> {
        self.note("exec")?;
        let mut cmd = Command::new("sh");
        let text = if args == ["codex", "--version"] {
            "printf 'codex-cli 0.155.1\\n'"
        } else if args == ["claude", "--version"] {
            if self.environment == "linux-rust-v3" && self.fail != "old_cli" {
                "printf '2.1.280 (Claude Code)\\n'"
            } else {
                "printf '2.1.278 (Claude Code)\\n'"
            }
        } else if args.first().map(String::as_str) == Some("git") {
            "true"
        } else if args.first().map(String::as_str) == Some("claude") {
            "cat >/dev/null; printf '%s\\n' '{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-exact-model\"}' '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}'"
        } else {
            "cat >/dev/null; printf '%s\\n' '{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}'"
        };
        cmd.args(["-c", text]);
        Ok(cmd)
    }
    fn import(&self, _: &str, _: &Path, _: &str) -> Result<()> {
        self.note("import")
    }
    fn stop(&self, _: &str) -> Result<()> {
        self.note("stop")
    }
    fn export(&self, _: &str, _: &str, dest: &Path, _: &Path, _: u64, _: usize) -> Result<()> {
        self.note("export")?;
        fs::create_dir(dest)?;
        fs::write(
            dest.join("Cargo.toml"),
            "[package]\nname='unfinished'\nversion='0.1.0'\n",
        )?;
        Ok(())
    }
    fn destroy(&self, _: &str) -> Result<()> {
        self.note("destroy")
    }
}
fn legacy_config() -> Config {
    Config {
        environment: "linux-rust-v2".into(),
        ..Config::default()
    }
}
fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join("tasks/spaceship-v1")).unwrap();
    fs::create_dir(repo.path().join("templates")).unwrap();
    fs::write(
        repo.path().join("tasks/spaceship-v1/prompt.md"),
        include_bytes!("../../tasks/spaceship-v1/prompt.md"),
    )
    .unwrap();
    fs::write(
        repo.path().join("tasks/spaceship-v1/task.toml"),
        include_bytes!("../../tasks/spaceship-v1/task.toml"),
    )
    .unwrap();
    fs::write(
        repo.path().join("templates/review.md"),
        include_bytes!("../../templates/review.md"),
    )
    .unwrap();
    fs::create_dir_all(repo.path().join("tasks/spaceship-v2")).unwrap();
    fs::write(
        repo.path().join("tasks/spaceship-v2/prompt.md"),
        include_bytes!("../../tasks/spaceship-v2/prompt.md"),
    )
    .unwrap();
    fs::write(
        repo.path().join("tasks/spaceship-v2/task.toml"),
        include_bytes!("../../tasks/spaceship-v2/task.toml"),
    )
    .unwrap();
    (repo, tempfile::tempdir().unwrap())
}
fn execute(fake: &Fake, repo: &Path, state: &Path) -> Result<String> {
    workflow::run(
        fake,
        &legacy_config(),
        repo,
        state,
        workflow::Attempt {
            agent: Agent::Codex,
            model: "exact-model",
            task_version: "spaceship-v1",
        },
        &AtomicBool::new(false),
    )
}
#[test]
fn prerequisite_failure_creates_no_attempt_and_delivers_no_prompt() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "preflight",
    };
    assert!(execute(&fake, repo.path(), state.path()).is_err());
    assert_eq!(*fake.calls.borrow(), ["preflight"]);
    assert_eq!(fs::read_dir(repo.path().join("runs")).unwrap().count(), 1); // allocation lock only
}

#[test]
fn pilot_archives_accepted_limits_without_claiming_strict_isolation() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "uncertified",
    };
    let config = Config {
        mode: bench::config::ExecutionMode::DockerPilot,
        ..legacy_config()
    };
    let id = workflow::run(
        &fake,
        &config,
        repo.path(),
        state.path(),
        workflow::Attempt {
            agent: Agent::Codex,
            model: "exact-model",
            task_version: "spaceship-v1",
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let (dir, run) = bench::protocol::load_run(repo.path(), &id).unwrap();
    assert_eq!(run.mode, config.mode);
    assert_eq!(run.protocol_version, "single-attempt-docker-pilot-v1");
    assert_eq!(
        run.accepted_limitations,
        config.mode.limitations(Agent::Codex)
    );
    assert!(!run.environment.isolation_verified);
    assert_eq!(run.outcome.completion, Completion::Normal);
    let replay: bench::replay::Replay =
        serde_json::from_slice(&fs::read(dir.join("replay.json")).unwrap()).unwrap();
    assert_eq!(replay.mode, config.mode);
    let mut mislabeled = run;
    mislabeled.protocol_version = bench::protocol::PROTOCOL.into();
    bench::protocol::atomic_json(&dir.join("run.json"), &mislabeled).unwrap();
    assert!(bench::protocol::load_run(repo.path(), &id).is_err());
}

#[test]
fn strict_workflow_still_rejects_uncertified_backend_before_prompt_delivery() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "uncertified",
    };
    assert!(execute(&fake, repo.path(), state.path()).is_err());
    assert!(!fake.calls.borrow().iter().any(|c| c == "exec"));
}

#[test]
fn pilot_runs_claude_and_archives_its_model_subscription_and_network_limitations() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "uncertified",
    };
    let config = Config {
        mode: bench::config::ExecutionMode::DockerPilot,
        ..legacy_config()
    };
    let id = workflow::run(
        &fake,
        &config,
        repo.path(),
        state.path(),
        workflow::Attempt {
            agent: Agent::Claude,
            model: "exact-model",
            task_version: "spaceship-v1",
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let (_, run) = bench::protocol::load_run(repo.path(), &id).unwrap();
    assert_eq!(run.agent.name, Agent::Claude);
    assert_eq!(run.agent.requested_model, "exact-model");
    assert_eq!(
        run.agent.reported_model.as_deref(),
        Some("claude-exact-model")
    );
    assert_eq!(
        run.agent.settings["authentication"]["billing"],
        "claude_subscription"
    );
    assert_eq!(
        run.accepted_limitations,
        config.mode.limitations(Agent::Claude)
    );
    assert!(!run.environment.isolation_verified);
    assert_eq!(run.outcome.completion, Completion::Normal);
    assert_eq!(run.cleanup, ArtifactStatus::Complete);
    assert_eq!(
        fake.calls
            .borrow()
            .iter()
            .filter(|c| *c == "create")
            .count(),
        1
    );
}
#[test]
fn normal_agent_completion_and_broken_game_are_separate() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "",
    };
    let id = execute(&fake, repo.path(), state.path()).unwrap();
    let (dir, run) = bench::protocol::load_run(repo.path(), &id).unwrap();
    assert_eq!(run.outcome.completion, Completion::Normal);
    assert_eq!(run.export.status, ArtifactStatus::Complete);
    assert_eq!(run.replay_preparation, ArtifactStatus::Failed); // no Cargo.lock; no repairs
    assert_eq!(run.cleanup, ArtifactStatus::Complete);
    let calls = fake.calls.borrow();
    assert!(
        calls.iter().position(|s| s == "stop").unwrap()
            < calls.iter().position(|s| s == "export").unwrap()
    );
    assert_eq!(calls.iter().filter(|s| s.as_str() == "create").count(), 1);
    assert!(
        state
            .path()
            .join("raw")
            .join(&id)
            .join("stdout.jsonl")
            .exists()
    );
    assert!(!dir.join("solution/Cargo.lock").exists());
    assert_eq!(
        fs::read(dir.join("prompt.md")).unwrap(),
        include_bytes!("../../tasks/spaceship-v1/prompt.md")
    );
}
#[test]
fn archived_task_is_frozen_but_new_version_is_editable() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "",
    };
    execute(&fake, repo.path(), state.path()).unwrap();
    fs::write(
        repo.path().join("tasks/spaceship-v1/prompt.md"),
        "changed\n",
    )
    .unwrap();
    let task = Task::load(repo.path(), "spaceship-v1").unwrap();
    assert!(
        RunStore::open(repo.path())
            .unwrap()
            .check_frozen(&task)
            .is_err()
    );
    fs::create_dir(repo.path().join("tasks/spaceship-v3")).unwrap();
    fs::write(
        repo.path().join("tasks/spaceship-v3/prompt.md"),
        "new version\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("tasks/spaceship-v3/task.toml"),
        include_str!("../../tasks/spaceship-v1/task.toml").replace("spaceship-v1", "spaceship-v3"),
    )
    .unwrap();
    let task = Task::load(repo.path(), "spaceship-v3").unwrap();
    RunStore::open(repo.path())
        .unwrap()
        .check_frozen(&task)
        .unwrap();
}
#[test]
fn playback_dimensions_come_from_the_verified_archive() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "",
    };
    let id = execute(&fake, repo.path(), state.path()).unwrap();
    fs::remove_dir_all(repo.path().join("tasks")).unwrap();
    let (dir, run) = bench::protocol::load_run(repo.path(), &id).unwrap();
    let contract = bench::protocol::archived_task_contract(&dir, &run).unwrap();
    assert_eq!((contract.columns, contract.rows), (124, 69));
    let original = fs::read_to_string(dir.join("task.toml")).unwrap();
    fs::write(
        dir.join("task.toml"),
        original.replace("columns = 124", "columns = 120"),
    )
    .unwrap();
    assert!(bench::protocol::archived_task_contract(&dir, &run).is_err());
    assert!(bench::protocol::load_run(repo.path(), &id).is_err());
}
#[test]
fn failures_destroy_guests_and_preserve_metadata() {
    for point in ["create", "verify", "exec", "stop", "export", "destroy"] {
        let (repo, state) = setup();
        let fake = Fake {
            calls: RefCell::new(vec![]),
            environment: "linux-rust-v2",
            fail: point,
        };
        let _ = execute(&fake, repo.path(), state.path());
        assert!(
            fake.calls.borrow().iter().any(|s| s == "destroy"),
            "cleanup missing after {point}"
        );
        let path = fs::read_dir(repo.path().join("runs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.is_dir())
            .unwrap();
        let run: Run = serde_json::from_slice(&fs::read(path.join("run.json")).unwrap()).unwrap();
        assert!(run.ended_at.is_some());
        assert!(run.export.status != ArtifactStatus::Pending);
        if ["create", "verify", "exec"].contains(&point) {
            assert_eq!(run.outcome.completion, Completion::InfrastructureFailure);
        }
    }
}

#[test]
fn stopped_snapshot_recovery_preserves_the_attempt_and_failure_history() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "export",
    };
    let id = execute(&fake, repo.path(), state.path()).unwrap();
    let (dir, before) = bench::protocol::load_run_metadata(repo.path(), &id).unwrap();
    assert_eq!(before.export.status, ArtifactStatus::Failed);
    let pack = |entries: &[(&str, &[u8])]| {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, data) in entries {
            let mut header = tar::Header::new_ustar();
            header.set_path(name).unwrap();
            header.set_mode(0o644);
            header.set_size(data.len() as u64);
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap()
    };
    let manifest = br#"[{"Layers":["layer.tar"]}]"#;
    let layer = pack(&[(
        "workspace/Cargo.toml",
        b"[package]\nname='unfinished'\nversion='0.1.0'\n",
    )]);
    let image = pack(&[("manifest.json", manifest), ("layer.tar", &layer)]);
    let snapshot = state.path().join("raw").join(&id).join("stopped-image.tar");
    fs::write(&snapshot, &image).unwrap();
    fake.calls.borrow_mut().clear();
    workflow::recover(
        &fake,
        &legacy_config(),
        repo.path(),
        state.path(),
        &id,
        &AtomicBool::new(false),
    )
    .unwrap();
    let (_, after) = bench::protocol::load_run(repo.path(), &id).unwrap();
    assert_eq!(after.started_at, before.started_at);
    assert_eq!(after.ended_at, before.ended_at);
    assert_eq!(after.prompt_sha256, before.prompt_sha256);
    assert_eq!(after.outcome.completion, Completion::Normal);
    assert_eq!(after.export.status, ArtifactStatus::Complete);
    assert_eq!(after.replay_preparation, ArtifactStatus::Failed); // missing lockfile is not repaired
    assert_eq!(*fake.calls.borrow(), ["preflight"]); // no generation or source execution
    let audit: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("export-recovery.json")).unwrap()).unwrap();
    assert_eq!(audit["previous_run"]["export"]["status"], "failed");
    assert_eq!(audit["snapshot_sha256"], archive::hash(&image));
    assert_eq!(fs::read(&snapshot).unwrap(), image);
    assert!(
        workflow::recover(
            &fake,
            &legacy_config(),
            repo.path(),
            state.path(),
            &id,
            &AtomicBool::new(false)
        )
        .is_err()
    );
}
#[test]
fn each_explicit_attempt_gets_new_id_and_no_previous_input() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "",
    };
    let a = execute(&fake, repo.path(), state.path()).unwrap();
    let b = execute(&fake, repo.path(), state.path()).unwrap();
    assert_ne!(a, b);
    assert_eq!(
        fake.calls
            .borrow()
            .iter()
            .filter(|s| s.as_str() == "create")
            .count(),
        2
    );
    assert!(!fake.calls.borrow().iter().any(|s| s == "import")); // no source/history input to generation
}
#[test]
fn edited_archives_fail_checksum_verification() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "",
    };
    let id = execute(&fake, repo.path(), state.path()).unwrap();
    let path = repo
        .path()
        .join("runs")
        .join(&id)
        .join("solution/Cargo.toml");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    fs::write(path, "tampered").unwrap();
    assert!(bench::protocol::load_run(repo.path(), &id).is_err());
}
#[test]
fn partial_create_is_guarded_and_never_reused() {
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "create",
    };
    assert!(Guest::new(&fake, Role::Playback).is_err());
    assert_eq!(*fake.calls.borrow(), ["create", "destroy"]);
}
#[test]
fn atomic_metadata_roundtrip_does_not_publish_raw_provider_text() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust-v2",
        fail: "",
    };
    let id = execute(&fake, repo.path(), state.path()).unwrap();
    let json = archive::read_regular(
        &repo.path().join("runs").join(id).join("run.json"),
        1024 * 1024,
    )
    .unwrap();
    let text = std::str::from_utf8(&json).unwrap();
    assert!(!text.contains(repo.path().to_str().unwrap()));
    assert!(!text.contains(state.path().to_str().unwrap()));
    assert!(text.contains("\"cost_usd\": null"));
}

#[test]
fn opus_55_is_rejected_on_old_environment_before_any_backend_operation() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        fail: "",
        environment: "linux-rust-v2",
    };
    let error = workflow::run(
        &fake,
        &legacy_config(),
        repo.path(),
        state.path(),
        workflow::Attempt {
            agent: Agent::Claude,
            model: "claude-opus-5-5",
            task_version: "spaceship-v1",
        },
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.to_string().contains("2.1.280"));
    assert!(fake.calls.borrow().is_empty());
    assert!(!repo.path().join("runs").exists());
}

#[test]
fn v3_records_new_claude_pin_and_refuses_an_old_runtime_before_delivery() {
    for fail in ["", "old_cli"] {
        let (repo, state) = setup();
        let fake = Fake {
            calls: RefCell::new(vec![]),
            fail,
            environment: "linux-rust-v3",
        };
        let result = workflow::run(
            &fake,
            &Config::default(),
            repo.path(),
            state.path(),
            workflow::Attempt {
                agent: Agent::Claude,
                model: "claude-opus-5-5",
                task_version: "spaceship-v2",
            },
            &AtomicBool::new(false),
        );
        if fail.is_empty() {
            let (_, run) = bench::protocol::load_run(repo.path(), &result.unwrap()).unwrap();
            assert_eq!(run.agent.cli_version.as_deref(), Some("2.1.280"));
            assert_eq!(run.environment.environment, "linux-rust-v3");
            assert_eq!(run.agent.requested_model, "claude-opus-5-5");
            assert_eq!(run.outcome.completion, Completion::Normal);
        } else {
            assert!(result.unwrap_err().to_string().contains("environment pin"));
            let dir = fs::read_dir(repo.path().join("runs"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p.is_dir())
                .unwrap();
            let run: Run =
                serde_json::from_slice(&fs::read(dir.join("run.json")).unwrap()).unwrap();
            assert!(run.started_at.is_none());
            assert_eq!(run.outcome.completion, Completion::InfrastructureFailure);
            assert!(
                !state
                    .path()
                    .join("raw")
                    .join(run.run_id)
                    .join("stdout.jsonl")
                    .exists()
            );
        }
    }
}
