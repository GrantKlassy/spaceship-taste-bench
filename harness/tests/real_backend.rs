//! Opt-in only. A blocked prerequisite FAILS; it is never reported as an isolation pass.
use bench::{
    config::Config,
    sandbox::{self, Sbx},
};
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
