//! Opt-in only. A blocked prerequisite FAILS; it is never reported as an isolation pass.
use bench::{
    config::Config,
    sandbox::{self, Sbx},
};

#[test]
#[ignore = "requires local sbx and crates.io; exports and replays a trusted vendored fixture, no model call"]
fn real_vendored_terminal_fixture_export() {
    use bench::{
        archive,
        config::ExecutionMode,
        process,
        sandbox::{Guest, Role, Sandbox},
    };
    use std::{fs, sync::atomic::AtomicBool};
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let state = bench::config::state_dir(repo).unwrap();
    let work = tempfile::Builder::new()
        .prefix("vendor-export-check-")
        .tempdir_in(&state)
        .unwrap()
        .keep();
    archive::private_dir(&work).unwrap();
    let source = work.join("input");
    fs::create_dir_all(source.join("src")).unwrap();
    fs::write(source.join("Cargo.toml"), "[package]\nname='vendored-terminal-fixture'\nversion='0.1.0'\nedition='2021'\n[dependencies]\ncrossterm='=0.28.1'\n").unwrap();
    fs::write(
        source.join("src/main.rs"),
        "fn main() { println!(\"terminal fixture\"); }\n",
    )
    .unwrap();
    let input = work.join("input.tar");
    archive::pack(&source, &input, 1024 * 1024, 100).unwrap();
    let config = Config {
        mode: ExecutionMode::DockerPilot,
        ..Config::default()
    };
    let backend = Sbx::new(repo, &config).unwrap();
    backend.preflight(Role::Package).unwrap();
    let guest = Guest::new(&backend, Role::Package).unwrap();
    backend.verify(&guest.name, Role::Package).unwrap();
    backend.import(&guest.name, &input, "/workspace").unwrap();
    let script = "set -eu\ncd /workspace\nmkdir .cargo\ncargo generate-lockfile\ncargo vendor --locked vendor > .cargo/config.toml\nprintf '\\n[net]\\noffline = true\\n' >> .cargo/config.toml\ndu -sb vendor\n";
    process::control_logged_abort(
        backend
            .exec(
                &guest.name,
                &["bash".into(), "-c".into(), script.into()],
                false,
            )
            .unwrap(),
        &work.join("vendor-log"),
        &AtomicBool::new(false),
    )
    .unwrap();
    backend.stop(&guest.name).unwrap();
    let result = backend.export(
        &guest.name,
        "workspace",
        &work.join("solution"),
        &work,
        config.limits.export_bytes,
        config.limits.export_files,
    );
    guest.destroy().unwrap();
    eprintln!(
        "Trusted fixture diagnostics (retained on failure): {}",
        work.display()
    );
    result.unwrap();
    let solution = work.join("solution");
    bench::replay::validate_submission(&solution).unwrap();
    let files = archive::inventory(
        &solution,
        config.limits.export_bytes,
        config.limits.export_files,
    )
    .unwrap();
    let run: bench::protocol::Run = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "protocol_version": config.mode.protocol(),
        "mode": config.mode, "accepted_limitations": config.mode.limitations(),
        "run_id": format!("fixture-{}", uuid::Uuid::new_v4().simple()),
        "task_version": "fixture", "prompt_sha256": "unused", "task_sha256": "unused", "input_sha256": {},
        "harness": {"commit": null,"dirty":null},
        "agent": {"name":"codex","cli_version":null,"requested_model":"fixture","reported_model":null,"invocation":[],"settings":{}},
        "environment": {"backend":"sbx","backend_version":null,"environment":config.environment,"image_digest":null,"rust":null,"architecture":null,"effective_limits":null,"network_policy":null,"isolation_verified":false},
        "requested_limits": config.limits, "allocated_at":chrono::Utc::now(),"started_at":null,"ended_at":null,"elapsed_seconds":null,
        "outcome": bench::agents::Outcome::default(),
        "export": {"status":"complete","reason":null,"tree_sha256":archive::tree_hash(&files).unwrap(),"files":files},
        "cleanup":"complete","replay_preparation":"pending"
    })).unwrap();
    let prepared = bench::replay::prepare(&backend, &config, &work, &run, &state).unwrap();
    assert_eq!(
        prepared.preparation,
        bench::protocol::ArtifactStatus::Complete
    );
    let source_tar = work.join("source.tar");
    archive::pack(
        &solution,
        &source_tar,
        config.limits.export_bytes,
        config.limits.export_files,
    )
    .unwrap();
    let replay_guest = Guest::new(&backend, Role::Playback).unwrap();
    backend.verify(&replay_guest.name, Role::Playback).unwrap();
    backend
        .import(&replay_guest.name, &source_tar, "/workspace")
        .unwrap();
    let bundle = state.join("bundles").join(prepared.local_bundle.unwrap());
    backend
        .import(&replay_guest.name, &bundle, "/replay")
        .unwrap();
    let output = process::control_logged_abort(
        backend
            .exec(
                &replay_guest.name,
                &[
                    "cargo".into(),
                    "--config".into(),
                    "/replay/config.toml".into(),
                    "run".into(),
                    "--release".into(),
                    "--frozen".into(),
                ],
                false,
            )
            .unwrap(),
        &work.join("playback-log"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap().trim(),
        "terminal fixture"
    );
    backend.stop(&replay_guest.name).unwrap();
    replay_guest.destroy().unwrap();
    fs::remove_file(bundle).unwrap();
    fs::remove_dir_all(work).unwrap();
}
#[test]
#[ignore = "requires installed, authenticated, certified local microVM backend; no billable model call"]
fn real_sentinels_freshness_network_and_offline_fixture() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let state = tempfile::tempdir().unwrap();
    let backend = Sbx::new(repo, &Config::default()).unwrap();
    sandbox::integration_check(&backend, repo, state.path()).unwrap();
}

#[test]
#[ignore = "requires a stopped sbx snapshot of the trusted tiny fixture; never executes source"]
fn real_stopped_snapshot_export() {
    let input = std::env::var_os("BENCH_TEST_SNAPSHOT").expect("set BENCH_TEST_SNAPSHOT");
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("solution");
    let files = bench::archive::extract_image_workspace(
        std::path::Path::new(&input),
        "workspace",
        &output,
        8 * 1024 * 1024 * 1024,
        1024 * 1024,
        100,
    )
    .unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny");
    assert_eq!(
        files,
        bench::archive::inventory(&fixture, 1024 * 1024, 100).unwrap()
    );
}

#[cfg(unix)]
#[test]
#[ignore = "requires local sbx; interrupts the harness during a trusted diagnostic, no model call"]
fn real_ctrl_c_cleans_up_owned_guest() {
    ctrl_c_at("Diagnostic guest: ");
}

#[cfg(unix)]
#[test]
#[ignore = "requires local sbx; interrupts trusted provisioning, no model call"]
fn real_ctrl_c_during_provisioning() {
    ctrl_c_at("Provisioning guest: ");
}

#[cfg(unix)]
#[test]
#[ignore = "requires local sbx; interrupts a trusted fixture build, no model call"]
fn real_ctrl_c_during_build() {
    ctrl_c_at("Building replay fixture: ");
}

#[cfg(unix)]
#[test]
#[ignore = "requires local sbx; interrupts a trusted stopped snapshot, no model call"]
fn real_ctrl_c_during_export() {
    ctrl_c_at("Snapshotting guest: ");
}

#[cfg(unix)]
fn ctrl_c_at(trigger: &str) {
    use std::{
        io::{BufRead, BufReader},
        process::Stdio,
        time::{Duration, Instant},
    };
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let work = tempfile::tempdir().unwrap();
    let config = Config {
        limits: bench::config::Limits {
            cpus: 2,
            memory_mib: 2048,
            // Full image layers exceed the tiny guest writable fixture size.
            disk_mib: 8192,
            ..Default::default()
        },
        ..Default::default()
    };
    let settings = work.path().join("test.toml");
    std::fs::write(&settings, toml::to_string(&config).unwrap()).unwrap();
    struct Cleanup {
        child: std::process::Child,
        guests: Vec<String>,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
            for name in &self.guests {
                let _ = bench::process::host_command(std::path::Path::new("sbx"))
                    .args(["rm", "--force", name])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
    let mut command =
        bench::process::host_command(std::path::Path::new(env!("CARGO_BIN_EXE_bench")));
    command
        .arg("--repo")
        .arg(repo)
        .arg("--config")
        .arg(settings)
        .arg("check-integration")
        .env("BENCH_STATE_DIR", work.path().join("state"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut process = Cleanup {
        child: command.spawn().unwrap(),
        guests: Vec::new(),
    };
    let stderr = process.child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut reached = false;
    let mut progress = std::collections::VecDeque::new();
    while Instant::now() < deadline {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(100)) {
            if progress.len() == 20 {
                progress.pop_front();
            }
            progress.push_back(line.clone());
            if let Some(name) = line.strip_prefix("Provisioning guest: ") {
                bench::config::parse_component(name).unwrap();
                assert!(name.starts_with("bench-") && name.len() < 50);
                process.guests.push(name.to_owned());
            }
            if line.starts_with(trigger) {
                reached = true;
                break;
            }
        } else if process.child.try_wait().unwrap().is_some() {
            break;
        }
    }
    assert!(
        reached && !process.guests.is_empty(),
        "diagnostic did not reach cancellation phase {trigger}: {progress:?}"
    );
    // SAFETY: PID belongs to the live child created just above; signal its
    // controller, never the daemon or a broad process group.
    assert_eq!(
        unsafe { libc::kill(process.child.id() as i32, libc::SIGINT) },
        0
    );
    let deadline = Instant::now() + Duration::from_secs(35);
    while process.child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        process
            .child
            .try_wait()
            .unwrap()
            .expect("harness did not handle Ctrl-C")
            .code(),
        Some(1)
    );
    reader.join().unwrap();
    let output = bench::process::host_command(std::path::Path::new("sbx"))
        .args(["ls", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let list: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        list["sandboxes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| !process
                .guests
                .iter()
                .any(|name| s["name"].as_str() == Some(name.as_str()))),
        "Ctrl-C left its guest behind"
    );
    let output = bench::process::host_command(std::path::Path::new("sbx"))
        .args(["template", "ls", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let list: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        list["images"].as_array().unwrap().iter().all(|image| {
            !process
                .guests
                .iter()
                .any(|name| image["tag"].as_str() == Some(name.as_str()))
        }),
        "Ctrl-C left a snapshot template behind"
    );
    process.guests.clear();
}
