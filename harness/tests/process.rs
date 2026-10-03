//! Trusted shell fixtures test transport, not agent or sandbox isolation.
use std::{
    fs,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use terminal_game_taste_bench::{
    agents::{Agent, Completion},
    process,
};
#[test]
fn prompt_is_written_once_byte_for_byte_and_raw_controls_stay_in_private_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let delivered = tmp.path().join("delivered");
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "cat >\"$1\"; printf '\\033[31munsafe stderr' >&2; printf '%s\\n' '{\"type\":\"turn.completed\"}'", "fixture"]).arg(&delivered);
    let prompt = b"exact prompt\n\nwith trailing spaces  \n";
    let (out, _) = process::agent_request(
        cmd,
        Agent::Codex,
        prompt,
        tmp.path(),
        1024,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(fs::read(delivered).unwrap(), prompt);
    assert_eq!(out.completion, Completion::Normal);
    assert!(
        fs::read(tmp.path().join("stderr.log"))
            .unwrap()
            .contains(&27)
    );
}
#[test]
fn log_size_is_bounded_without_ending_the_agent_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "cat >/dev/null; printf '%s\\n' '{\"type\":\"item.completed\",\"text\":\"long content\"}' '{\"type\":\"turn.completed\"}'"]);
    let (out, _) = process::agent_request(
        cmd,
        Agent::Codex,
        b"task",
        tmp.path(),
        10,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(out.completion, Completion::Normal);
    assert!(out.transcript_truncated);
    assert_eq!(
        fs::metadata(tmp.path().join("stdout.jsonl")).unwrap().len(),
        10
    );
}
#[test]
fn abort_interrupts_even_a_blocked_prompt_write() {
    let tmp = tempfile::tempdir().unwrap();
    let abort = Arc::new(AtomicBool::new(false));
    let signal = abort.clone();
    let sender = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        signal.store(true, Ordering::SeqCst);
    });
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "sleep 10"]);
    let (out, _) = process::agent_request(
        cmd,
        Agent::Codex,
        &vec![b'x'; 1024 * 1024],
        tmp.path(),
        1024,
        &abort,
    )
    .unwrap();
    sender.join().unwrap();
    assert_eq!(out.completion, Completion::UserAbort);
}

#[test]
fn setup_failure_retains_bounded_private_diagnostics_without_rendering_them() {
    let work = tempfile::tempdir().unwrap();
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "printf 'private-account\\033[31m' >&2; head -c 100000 /dev/zero >&2; exit 7",
    ]);
    let error = process::control_logged(command, work.path())
        .unwrap_err()
        .to_string();
    assert!(!error.contains("private-account") && !error.contains('\x1b'));
    let folder = fs::read_dir(work.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = folder.join("stderr.log");
    let bytes = fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"private-account\x1b[31m"));
    assert_eq!(bytes.len(), 65536);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn cancellation_reaps_transport_descendants_and_retains_bounded_logs() {
    let work = tempfile::tempdir().unwrap();
    let abort = Arc::new(AtomicBool::new(false));
    let signal = abort.clone();
    let sender = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        signal.store(true, Ordering::SeqCst);
    });
    let mut command = Command::new("sh");
    // The shell exits while its descendant still holds both pipes open.
    command.args([
        "-c",
        "printf 'started\\n'; printf 'private diagnostic' >&2; sleep 10 & exit 0",
    ]);
    let start = std::time::Instant::now();
    let error = process::control_logged_abort(command, work.path(), &abort)
        .unwrap_err()
        .to_string();
    sender.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(error.contains("aborted") && !error.contains("private diagnostic"));
    let folder = fs::read_dir(work.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read(folder.join("stdout.log")).unwrap(), b"started\n");
    assert_eq!(
        fs::read(folder.join("stderr.log")).unwrap(),
        b"private diagnostic"
    );
}

#[test]
fn pre_cancelled_jobs_never_start_and_cleanup_commands_still_work() {
    let work = tempfile::tempdir().unwrap();
    let marker = work.path().join("must-not-exist");
    let mut command = Command::new("touch");
    command.arg(&marker);
    assert!(process::guest_job(command, work.path(), &AtomicBool::new(true)).is_err());
    assert!(!marker.exists());
    let mut cleanup = Command::new("touch");
    cleanup.arg(&marker);
    process::control_logged(cleanup, work.path()).unwrap();
    assert!(marker.exists());
}

#[test]
fn oversized_administrative_output_is_drained_and_rejected() {
    let work = tempfile::tempdir().unwrap();
    let mut command = Command::new("head");
    command.args(["-c", "5000000", "/dev/zero"]);
    let error = process::control_logged(command, work.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("oversized"));
    let folder = fs::read_dir(work.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        fs::metadata(folder.join("stdout.log")).unwrap().len(),
        65536
    );
}
