//! Simulated control flow only: these tests make no claim about real VM isolation.
use anyhow::{Result, bail};
use serde_json::json;
use std::{cell::RefCell, fs, path::Path, process::Command, sync::atomic::AtomicBool};
use terminal_game_taste_bench::{
    agents::{Agent, Completion},
    archive,
    config::Config,
    protocol::{ArtifactStatus, EnvironmentIdentity, Run, RunStore, Task},
    sandbox::{Guest, Role, Sandbox},
    workflow,
};

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
            effective_limits: Some(Config::default().limits),
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
            if self.fail != "old_cli" {
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
fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join("tasks/spaceship")).unwrap();
    fs::create_dir(repo.path().join("templates")).unwrap();
    fs::write(
        repo.path().join("tasks/spaceship/prompt.md"),
        include_bytes!("../../tasks/spaceship/prompt.md"),
    )
    .unwrap();
    fs::write(
        repo.path().join("tasks/spaceship/task.toml"),
        include_bytes!("../../tasks/spaceship/task.toml"),
    )
    .unwrap();
    fs::write(
        repo.path().join("templates/review.md"),
        include_bytes!("../../templates/review.md"),
    )
    .unwrap();
    (repo, tempfile::tempdir().unwrap())
}
fn execute(fake: &Fake, repo: &Path, state: &Path) -> Result<String> {
    workflow::run(
        fake,
        &Config::default(),
        repo,
        state,
        workflow::Attempt {
            agent: Agent::Codex,
            model: "exact-model",
            task_name: "spaceship",
        },
        &AtomicBool::new(false),
    )
}
#[test]
fn prerequisite_failure_creates_no_attempt_and_delivers_no_prompt() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust",
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
        environment: "linux-rust",
        fail: "uncertified",
    };
    let config = Config {
        mode: terminal_game_taste_bench::config::ExecutionMode::DockerPilot,
        ..Config::default()
    };
    let id = workflow::run(
        &fake,
        &config,
        repo.path(),
        state.path(),
        workflow::Attempt {
            agent: Agent::Codex,
            model: "exact-model",
            task_name: "spaceship",
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let (dir, run) = terminal_game_taste_bench::protocol::load_run(repo.path(), &id).unwrap();
    assert_eq!(run.mode, config.mode);
    assert_eq!(run.protocol_version, "single-attempt-docker-pilot-v1");
    assert_eq!(
        run.accepted_limitations,
        config.mode.limitations(Agent::Codex)
    );
    assert!(!run.environment.isolation_verified);
    assert_eq!(run.outcome.completion, Completion::Normal);
    let replay: terminal_game_taste_bench::replay::Replay =
        serde_json::from_slice(&fs::read(dir.join("replay.json")).unwrap()).unwrap();
    assert_eq!(replay.mode, config.mode);
    let mut mislabeled = run;
    mislabeled.protocol_version = terminal_game_taste_bench::protocol::PROTOCOL.into();
    terminal_game_taste_bench::protocol::atomic_json(&dir.join("run.json"), &mislabeled).unwrap();
    assert!(terminal_game_taste_bench::protocol::load_run(repo.path(), &id).is_err());
}

#[test]
fn strict_workflow_still_rejects_uncertified_backend_before_prompt_delivery() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust",
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
        environment: "linux-rust",
        fail: "uncertified",
    };
    let config = Config {
        mode: terminal_game_taste_bench::config::ExecutionMode::DockerPilot,
        ..Config::default()
    };
    let id = workflow::run(
        &fake,
        &config,
        repo.path(),
        state.path(),
        workflow::Attempt {
            agent: Agent::Claude,
            model: "exact-model",
            task_name: "spaceship",
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let (_, run) = terminal_game_taste_bench::protocol::load_run(repo.path(), &id).unwrap();
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
        environment: "linux-rust",
        fail: "",
    };
    let id = execute(&fake, repo.path(), state.path()).unwrap();
    let (dir, run) = terminal_game_taste_bench::protocol::load_run(repo.path(), &id).unwrap();
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
        include_bytes!("../../tasks/spaceship/prompt.md")
    );
}
#[test]
fn attempts_preserve_selected_tasks_without_a_spaceship_directory() {
    for name in ["smoke", "maze"] {
        let repo = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let task_dir = repo.path().join("tasks").join(name);
        fs::create_dir_all(&task_dir).unwrap();
        fs::create_dir(repo.path().join("templates")).unwrap();
        fs::write(
            repo.path().join("templates/review.md"),
            include_bytes!("../../templates/review.md"),
        )
        .unwrap();
        let mut contract: terminal_game_taste_bench::protocol::TaskContract =
            toml::from_str(include_str!("../../tasks/smoke/task.toml")).unwrap();
        let prompt = if name == "smoke" {
            include_bytes!("../../tasks/smoke/prompt.md").as_slice()
        } else {
            contract.name = name.into();
            contract.columns = 96;
            contract.rows = 32;
            b"Create a terminal maze game.\n".as_slice()
        };
        fs::write(
            task_dir.join("task.toml"),
            toml::to_string(&contract).unwrap(),
        )
        .unwrap();
        fs::write(task_dir.join("prompt.md"), prompt).unwrap();
        let fake = Fake {
            calls: RefCell::new(vec![]),
            environment: "linux-rust",
            fail: "",
        };
        let id = workflow::run(
            &fake,
            &Config::default(),
            repo.path(),
            state.path(),
            workflow::Attempt {
                agent: Agent::Codex,
                model: "exact-model",
                task_name: name,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        let (dir, run) = terminal_game_taste_bench::protocol::load_run(repo.path(), &id).unwrap();
        assert_eq!(run.task_name, name);
        assert_eq!(fs::read(dir.join("prompt.md")).unwrap(), prompt);
        assert_eq!(run.prompt_sha256, archive::hash(prompt));
        let archived =
            terminal_game_taste_bench::protocol::archived_task_contract(&dir, &run).unwrap();
        assert_eq!(
            (archived.columns, archived.rows),
            (contract.columns, contract.rows)
        );
        assert_eq!(run.outcome.completion, Completion::Normal);
        assert_eq!(run.cleanup, ArtifactStatus::Complete);
    }
}
#[test]
fn editing_current_inputs_preserves_existing_attempt_snapshots() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust",
        fail: "",
    };
    let lock_path = repo
        .path()
        .join("environments/linux-rust/environment.lock.json");
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let original_lock = include_bytes!("../../environments/linux-rust/environment.lock.json");
    fs::write(&lock_path, original_lock).unwrap();
    let first = execute(&fake, repo.path(), state.path()).unwrap();
    let (first_dir, first_run) =
        terminal_game_taste_bench::protocol::load_run(repo.path(), &first).unwrap();
    let before = archive::inventory(&first_dir, 1_000_000, 100).unwrap();

    fs::write(repo.path().join("tasks/spaceship/prompt.md"), "changed\n").unwrap();
    let contract =
        include_str!("../../tasks/spaceship/task.toml").replace("columns = 124", "columns = 100");
    fs::write(repo.path().join("tasks/spaceship/task.toml"), contract).unwrap();
    let mut updated_lock: serde_json::Value = serde_json::from_slice(original_lock).unwrap();
    updated_lock["images"]["base"]["image_id"] = json!(format!("sha256:{}", "b".repeat(64)));
    fs::write(&lock_path, serde_json::to_vec(&updated_lock).unwrap()).unwrap();
    let second = workflow::run(
        &fake,
        &Config::default(),
        repo.path(),
        state.path(),
        workflow::Attempt {
            agent: Agent::Codex,
            model: "another-model",
            task_name: "spaceship",
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let (second_dir, second_run) =
        terminal_game_taste_bench::protocol::load_run(repo.path(), &second).unwrap();
    assert_eq!(first_run.task_name, second_run.task_name);
    assert_ne!(first_run.prompt_sha256, second_run.prompt_sha256);
    assert_ne!(first_run.task_sha256, second_run.task_sha256);
    assert_ne!(
        first_run.input_sha256["environment.lock.json"],
        second_run.input_sha256["environment.lock.json"]
    );
    assert_eq!(
        fs::read(second_dir.join("prompt.md")).unwrap(),
        b"changed\n"
    );
    assert_eq!(
        terminal_game_taste_bench::protocol::archived_task_contract(&first_dir, &first_run)
            .unwrap()
            .columns,
        124
    );
    assert_eq!(
        terminal_game_taste_bench::protocol::archived_task_contract(&second_dir, &second_run)
            .unwrap()
            .columns,
        100
    );
    assert_eq!(
        before,
        archive::inventory(&first_dir, 1_000_000, 100).unwrap()
    );
    RunStore::open(repo.path())
        .unwrap()
        .verify_archives()
        .unwrap();
}
#[test]
fn allocation_rejects_tampered_archived_inputs() {
    for file in ["prompt.md", "task.toml", "settings.json"] {
        let (repo, state) = setup();
        let fake = Fake {
            calls: RefCell::new(vec![]),
            environment: "linux-rust",
            fail: "",
        };
        let id = execute(&fake, repo.path(), state.path()).unwrap();
        fs::write(repo.path().join("runs").join(id).join(file), "tampered").unwrap();
        fake.calls.borrow_mut().clear();
        assert!(execute(&fake, repo.path(), state.path()).is_err());
        assert!(fake.calls.borrow().is_empty());
    }
}
#[test]
fn playback_dimensions_come_from_the_verified_archive() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust",
        fail: "",
    };
    let id = execute(&fake, repo.path(), state.path()).unwrap();
    fs::remove_dir_all(repo.path().join("tasks")).unwrap();
    let (dir, run) = terminal_game_taste_bench::protocol::load_run(repo.path(), &id).unwrap();
    let contract = terminal_game_taste_bench::protocol::archived_task_contract(&dir, &run).unwrap();
    assert_eq!((contract.columns, contract.rows), (124, 69));
    let original = fs::read_to_string(dir.join("task.toml")).unwrap();
    fs::write(
        dir.join("task.toml"),
        original.replace("columns = 124", "columns = 120"),
    )
    .unwrap();
    assert!(terminal_game_taste_bench::protocol::archived_task_contract(&dir, &run).is_err());
    assert!(terminal_game_taste_bench::protocol::load_run(repo.path(), &id).is_err());
}
#[test]
fn failures_destroy_guests_and_preserve_metadata() {
    for point in ["create", "verify", "exec", "stop", "export", "destroy"] {
        let (repo, state) = setup();
        let fake = Fake {
            calls: RefCell::new(vec![]),
            environment: "linux-rust",
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
        environment: "linux-rust",
        fail: "export",
    };
    let id = execute(&fake, repo.path(), state.path()).unwrap();
    let (dir, before) =
        terminal_game_taste_bench::protocol::load_run_metadata(repo.path(), &id).unwrap();
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
        &Config::default(),
        repo.path(),
        state.path(),
        &id,
        &AtomicBool::new(false),
    )
    .unwrap();
    let (_, after) = terminal_game_taste_bench::protocol::load_run(repo.path(), &id).unwrap();
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
            &Config::default(),
            repo.path(),
            state.path(),
            &id,
            &AtomicBool::new(false)
        )
        .is_err()
    );
}
#[test]
fn different_models_get_separate_attempts_and_no_previous_input() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust",
        fail: "",
    };
    let a = execute(&fake, repo.path(), state.path()).unwrap();
    let b = workflow::run(
        &fake,
        &Config::default(),
        repo.path(),
        state.path(),
        workflow::Attempt {
            agent: Agent::Codex,
            model: "another-model",
            task_name: "spaceship",
        },
        &AtomicBool::new(false),
    )
    .unwrap();
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
fn duplicate_daily_id_preserves_the_existing_archive() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust",
        fail: "",
    };
    let id = execute(&fake, repo.path(), state.path()).unwrap();
    let (dir, run) = terminal_game_taste_bench::protocol::load_run(repo.path(), &id).unwrap();
    let before = archive::tree_hash(&archive::inventory(&dir, 1_000_000, 100).unwrap()).unwrap();
    let task = Task::load(repo.path(), "spaceship").unwrap();
    let store = RunStore::open(repo.path()).unwrap();
    let error = store
        .allocate(&run, &task, b"replacement review", &Default::default())
        .unwrap_err();
    assert!(error.to_string().contains(&id));
    assert!(
        error
            .to_string()
            .contains("only one run per agent/model and UTC date")
    );
    let after = archive::tree_hash(&archive::inventory(&dir, 1_000_000, 100).unwrap()).unwrap();
    assert_eq!(before, after);
    assert_eq!(
        fs::read_dir(&store.root)
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path().is_dir())
            .count(),
        1
    );
}
#[test]
fn edited_archives_fail_checksum_verification() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust",
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
    assert!(terminal_game_taste_bench::protocol::load_run(repo.path(), &id).is_err());
}
#[test]
fn partial_create_is_guarded_and_never_reused() {
    let fake = Fake {
        calls: RefCell::new(vec![]),
        environment: "linux-rust",
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
        environment: "linux-rust",
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
fn claude_pin_is_recorded_and_mismatched_runtime_is_rejected_before_delivery() {
    for fail in ["", "old_cli"] {
        let (repo, state) = setup();
        let fake = Fake {
            calls: RefCell::new(vec![]),
            fail,
            environment: "linux-rust",
        };
        let result = workflow::run(
            &fake,
            &Config::default(),
            repo.path(),
            state.path(),
            workflow::Attempt {
                agent: Agent::Claude,
                model: "claude-opus-5-5",
                task_name: "spaceship",
            },
            &AtomicBool::new(false),
        );
        if fail.is_empty() {
            let (_, run) =
                terminal_game_taste_bench::protocol::load_run(repo.path(), &result.unwrap())
                    .unwrap();
            assert_eq!(run.agent.cli_version.as_deref(), Some("2.1.280"));
            assert_eq!(run.environment.environment, "linux-rust");
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
