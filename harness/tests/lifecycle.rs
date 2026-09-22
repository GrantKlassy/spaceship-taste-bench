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
            environment: Config::default().environment,
            image_digest: Some(format!("sha256:{}", "a".repeat(64))),
            rust: Some("1.97.0".into()),
            architecture: Some("x86_64".into()),
            effective_limits: Some(Config::default().limits),
            network_policy: Some(json!({"fixture": true})),
            isolation_verified: true,
        })
    }
    fn exec(&self, _: &str, args: &[String], _: bool) -> Result<Command> {
        self.note("exec")?;
        let mut cmd = Command::new("sh");
        let text = if args == ["codex", "--version"] {
            "printf 'codex-cli 0.155.1\\n'"
        } else if args.first().map(String::as_str) == Some("git") {
            "true"
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
        fail: "preflight",
    };
    assert!(execute(&fake, repo.path(), state.path()).is_err());
    assert_eq!(*fake.calls.borrow(), ["preflight"]);
    assert_eq!(fs::read_dir(repo.path().join("runs")).unwrap().count(), 1); // allocation lock only
}
#[test]
fn normal_agent_completion_and_broken_game_are_separate() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
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
    fs::create_dir(repo.path().join("tasks/spaceship-v2")).unwrap();
    fs::write(
        repo.path().join("tasks/spaceship-v2/prompt.md"),
        "new version\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("tasks/spaceship-v2/task.toml"),
        include_str!("../../tasks/spaceship-v1/task.toml").replace("spaceship-v1", "spaceship-v2"),
    )
    .unwrap();
    let task = Task::load(repo.path(), "spaceship-v2").unwrap();
    RunStore::open(repo.path())
        .unwrap()
        .check_frozen(&task)
        .unwrap();
}
#[test]
fn failures_destroy_guests_and_preserve_metadata() {
    for point in ["create", "verify", "exec", "stop", "export", "destroy"] {
        let (repo, state) = setup();
        let fake = Fake {
            calls: RefCell::new(vec![]),
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
fn each_explicit_attempt_gets_new_id_and_no_previous_input() {
    let (repo, state) = setup();
    let fake = Fake {
        calls: RefCell::new(vec![]),
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
