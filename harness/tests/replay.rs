//! Replay controller tests use trusted data and a simulated VM, never host Cargo.
use anyhow::Result;
use bench::{
    archive,
    config::Config,
    protocol::{ArtifactStatus, EnvironmentIdentity, Run},
    replay,
    sandbox::{Role, Sandbox},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct PackageFixture {
    source: PathBuf,
    mutate_copy: bool,
}
impl Sandbox for PackageFixture {
    fn preflight(&self, role: Role) -> Result<()> {
        assert_eq!(role, Role::Package);
        Ok(())
    }
    fn create(&self, _: &str, _: Role) -> Result<()> {
        Ok(())
    }
    fn verify(&self, _: &str, _: Role) -> Result<EnvironmentIdentity> {
        Ok(EnvironmentIdentity {
            backend: "test-only".into(),
            backend_version: None,
            environment: "linux-rust".into(),
            image_digest: Some(format!("sha256:{}", "a".repeat(64))),
            rust: Some("1.97.0".into()),
            architecture: None,
            effective_limits: Some(Config::default().limits),
            network_policy: Some(serde_json::json!({"test_only": true})),
            isolation_verified: true,
        })
    }
    fn exec(&self, _: &str, args: &[String], _: bool) -> Result<Command> {
        assert!(
            args.last()
                .unwrap()
                .contains("cargo vendor --locked --versioned-dirs")
        );
        // No generated source is compiled/executed. Export below supplies known fixture data.
        Ok(Command::new("true"))
    }
    fn import(&self, _: &str, _: &Path, dest: &str) -> Result<()> {
        assert_eq!(dest, "/workspace");
        Ok(())
    }
    fn stop(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn export(&self, _: &str, prefix: &str, dest: &Path, _: &Path, _: u64, _: usize) -> Result<()> {
        fs::create_dir(dest)?;
        if prefix == "replay" {
            fs::create_dir(dest.join("vendor"))?;
            fs::write(dest.join("config.toml"), "")?;
        } else {
            for entry in walkdir::WalkDir::new(&self.source).min_depth(1) {
                let entry = entry?;
                let target = dest.join(entry.path().strip_prefix(&self.source)?);
                if entry.file_type().is_dir() {
                    fs::create_dir(&target)?;
                } else {
                    fs::copy(entry.path(), target)?;
                }
            }
            if self.mutate_copy {
                fs::write(dest.join("Cargo.lock"), "changed by packaging")?;
            }
        }
        Ok(())
    }
    fn destroy(&self, _: &str) -> Result<()> {
        Ok(())
    }
}
fn setup() -> (tempfile::TempDir, tempfile::TempDir, Run) {
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let solution = dir.path().join("solution");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny");
    fs::create_dir(&solution).unwrap();
    fs::create_dir(solution.join("src")).unwrap();
    for name in ["Cargo.toml", "Cargo.lock", "src/main.rs"] {
        fs::copy(fixture.join(name), solution.join(name)).unwrap();
    }
    let files = archive::inventory(&solution, 100000, 100).unwrap();
    let run: Run = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "protocol_version": "single-attempt-v1", "run_id": "test-fixture",
        "task_name": "fixture", "prompt_sha256": "unused", "task_sha256": "unused", "input_sha256": {},
        "harness": {"commit": null,"dirty":null},
        "agent": {"name":"codex","cli_version":null,"requested_model":"fixture","reported_model":null,"invocation":[],"settings":{}},
        "environment": {"backend":"test-only","backend_version":null,"environment":"linux-rust","image_digest":null,"rust":null,"architecture":null,"effective_limits":null,"network_policy":null,"isolation_verified":false},
        "requested_limits": Config::default().limits, "allocated_at":"2026-09-21T19:00:00Z","started_at":null,"ended_at":null,"elapsed_seconds":null,
        "outcome": bench::agents::Outcome::default(),
        "export": {"status":"complete","reason":null,"tree_sha256":archive::tree_hash(&files).unwrap(),"files":files},
        "cleanup":"complete","replay_preparation":"pending"
    })).unwrap();
    (dir, state, run)
}
#[test]
fn package_has_checksum_source_binding_runtime_and_separate_configuration() {
    let (dir, state, run) = setup();
    let backend = PackageFixture {
        source: dir.path().join("solution"),
        mutate_copy: false,
    };
    let prepared =
        replay::prepare(&backend, &Config::default(), dir.path(), &run, state.path()).unwrap();
    assert_eq!(prepared.preparation, ArtifactStatus::Complete);
    assert_eq!(prepared.launch, ArtifactStatus::Unavailable);
    assert_eq!(prepared.source_tree_sha256, run.export.tree_sha256);
    assert!(prepared.environment_image.is_some());
    let bundle = state
        .path()
        .join("bundles")
        .join(prepared.local_bundle.unwrap());
    assert_eq!(
        archive::file_hash(&bundle, 100000).unwrap(),
        prepared.bundle_sha256.unwrap()
    );
    assert!(!dir.path().join("solution/.cargo/config.toml").exists());
    assert_eq!(
        archive::inventory(&dir.path().join("solution"), 100000, 100).unwrap(),
        run.export.files
    );
}
#[test]
fn packaging_that_changes_source_is_rejected_without_repairing_the_archive() {
    let (dir, state, run) = setup();
    let backend = PackageFixture {
        source: dir.path().join("solution"),
        mutate_copy: true,
    };
    let prepared =
        replay::prepare(&backend, &Config::default(), dir.path(), &run, state.path()).unwrap();
    assert_eq!(prepared.preparation, ArtifactStatus::Failed);
    assert!(prepared.local_bundle.is_none());
    assert_eq!(
        archive::inventory(&dir.path().join("solution"), 100000, 100).unwrap(),
        run.export.files
    );
}

#[test]
fn replay_requires_the_archived_mode_before_creating_a_guest() {
    let (dir, state, run) = setup();
    let backend = PackageFixture {
        source: dir.path().join("solution"),
        mutate_copy: false,
    };
    let config = Config {
        mode: bench::config::ExecutionMode::DockerPilot,
        ..Config::default()
    };
    assert!(replay::prepare(&backend, &config, dir.path(), &run, state.path()).is_err());
    assert!(
        replay::play(
            &backend,
            &config,
            dir.path(),
            &run,
            state.path(),
            &std::sync::atomic::AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(!dir.path().join("replay.json").exists());
}
