//! Host process handling. No shell interpolation; generated code never reaches this module as a host command.
use crate::agents::{Agent, Events, Outcome};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub fn host_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_clear();
    // Backend-side authentication can use the real OS keychain. None of these are
    // sent to a guest by the harness. In particular no API, cloud, or SSH variables.
    for key in [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "TERM",
        "LANG",
        "LC_ALL",
    ] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    cmd.current_dir(std::env::temp_dir());
    cmd
}

pub fn signals() -> Result<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    let copy = flag.clone();
    ctrlc::set_handler(move || {
        copy.store(true, Ordering::SeqCst);
    })?;
    Ok(flag)
}
fn separate_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
}
fn kill(child: &mut Child) {
    #[cfg(unix)]
    {
        // SAFETY: this is a child process group created by Command::process_group.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            kill(&mut self.0);
        }
    }
}
pub(crate) fn private_file(path: &Path) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    Ok(opts.open(path)?)
}

struct Captured {
    events: Events,
    truncated: bool,
    io_failed: bool,
}
fn drain(mut input: impl Read, mut output: File, limit: u64, agent: Option<Agent>) -> Captured {
    let mut events = Events::default();
    let mut written = 0u64;
    let mut truncated = false;
    let mut io_failed = false;
    let mut pending = Vec::new();
    let mut overlong = false;
    let mut buf = [0; 8192];
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => {
                io_failed = true;
                break;
            }
        };
        let to_write = (limit - written).min(n as u64) as usize;
        if output.write_all(&buf[..to_write]).is_err() {
            io_failed = true;
        }
        written += to_write as u64;
        truncated |= to_write < n;
        if let Some(agent) = agent {
            for &byte in &buf[..n] {
                if byte == b'\n' {
                    if !pending.is_empty() && !overlong {
                        events.consume(agent, &pending);
                    }
                    pending.clear();
                    overlong = false;
                } else if pending.len() < 1024 * 1024 && !overlong {
                    pending.push(byte);
                } else {
                    overlong = true;
                    events.malformed = true;
                    pending.clear();
                }
            }
        }
    }
    if let Some(agent) = agent
        && !pending.is_empty()
        && !overlong
    {
        events.consume(agent, &pending);
    }
    if output.sync_all().is_err() {
        io_failed = true;
    }
    Captured {
        events,
        truncated,
        io_failed,
    }
}

/// No deadline on the agent's natural tool loop. Log overflow truncates capture,
/// keeps parsing and draining, and is recorded; it does not end the attempt.
pub fn agent_request(
    mut cmd: Command,
    agent: Agent,
    prompt: &[u8],
    raw: &Path,
    limit: u64,
    abort: &AtomicBool,
) -> Result<(Outcome, Option<String>)> {
    let stdout = private_file(&raw.join("stdout.jsonl"))?;
    let stderr = private_file(&raw.join("stderr.log"))?;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    separate_group(&mut cmd);
    let mut child = ChildGuard(
        cmd.spawn()
            .context("could not start sandbox agent transport")?,
    );
    let out = child.0.stdout.take().context("missing stdout pipe")?;
    let err = child.0.stderr.take().context("missing stderr pipe")?;
    let out_thread = thread::spawn(move || drain(out, stdout, limit, Some(agent)));
    let err_thread = thread::spawn(move || drain(err, stderr, limit, None));
    // One write, then EOF. No response to questions and no resume/retry path.
    let mut input = child.0.stdin.take().context("missing stdin pipe")?;
    let bytes = prompt.to_vec();
    let input_thread = thread::spawn(move || input.write_all(&bytes).is_ok());
    let mut last_progress = Instant::now();
    let mut aborted = false;
    let exit = loop {
        if abort.load(Ordering::SeqCst) {
            aborted = true;
            kill(&mut child.0);
            break child.0.try_wait()?.and_then(|s| s.code());
        }
        if let Some(status) = child.0.try_wait()? {
            break status.code();
        }
        if last_progress.elapsed() >= Duration::from_secs(30) {
            eprintln!("Agent request is still running; waiting for natural completion.");
            last_progress = Instant::now();
        }
        thread::sleep(Duration::from_millis(100));
    };
    let out = out_thread
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader failed"))?;
    let err = err_thread
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader failed"))?;
    let input_ok = input_thread.join().unwrap_or(false);
    Ok(out.events.finish(
        exit,
        aborted,
        out.io_failed || err.io_failed || (!input_ok && exit == Some(0)),
        out.truncated || err.truncated,
    ))
}

/// Bounded administrative request, never used to time-limit an agent attempt.
pub fn control(cmd: Command) -> Result<Vec<u8>> {
    let (exit, data) = control_output(cmd)?;
    ensure!(
        exit == Some(0),
        "backend administrative command failed (raw output withheld)"
    );
    Ok(data)
}

/// Keep bounded administrative stdout/stderr outside the checkout. Filenames
/// contain only random IDs; command arguments and authentication input are not logged.
pub fn control_logged(cmd: Command, logs: &Path) -> Result<Vec<u8>> {
    let (exit, data) = control_output_logged(cmd, logs)?;
    ensure!(
        exit == Some(0),
        "backend setup command failed; inspect private backend/setup logs"
    );
    Ok(data)
}
pub(crate) fn control_output_logged(cmd: Command, logs: &Path) -> Result<(Option<i32>, Vec<u8>)> {
    control_output_logged_inner(cmd, logs, None, Some(Duration::from_secs(30)))
}

pub fn control_logged_abort(cmd: Command, logs: &Path, abort: &AtomicBool) -> Result<Vec<u8>> {
    let (exit, data) = control_output_logged_abort(cmd, logs, abort)?;
    ensure!(
        exit == Some(0),
        "backend setup command failed; inspect private backend/setup logs"
    );
    Ok(data)
}

pub(crate) fn control_output_logged_abort(
    cmd: Command,
    logs: &Path,
    abort: &AtomicBool,
) -> Result<(Option<i32>, Vec<u8>)> {
    control_output_logged_inner(cmd, logs, Some(abort), Some(Duration::from_secs(30)))
}

/// Guest builds/vendoring are cancellable but have no arbitrary 30-second budget.
/// Killing the transport does not stop guest work; the caller's Guest guard does.
pub fn guest_job(cmd: Command, logs: &Path, abort: &AtomicBool) -> Result<Vec<u8>> {
    let (exit, data) = control_output_logged_inner(cmd, logs, Some(abort), None)?;
    ensure!(
        exit == Some(0),
        "guest build or packaging failed; inspect private setup logs"
    );
    Ok(data)
}

fn control_output_logged_inner(
    cmd: Command,
    logs: &Path,
    abort: Option<&AtomicBool>,
    timeout: Option<Duration>,
) -> Result<(Option<i32>, Vec<u8>)> {
    ensure!(
        !abort.is_some_and(|a| a.load(Ordering::SeqCst)),
        "backend operation aborted before launch"
    );
    std::fs::create_dir_all(logs)?;
    crate::archive::ensure_directory(logs)?;
    let folder = logs.join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir(&folder)?;
    crate::archive::private_dir(&folder)?;
    let stdout = private_file(&folder.join("stdout.log"))?;
    let stderr = private_file(&folder.join("stderr.log"))?;
    control_output_inner(cmd, Some((stdout, stderr)), abort, timeout)
}

/// Diagnostics can return useful structured results with a nonzero exit status.
/// Never render raw diagnostic fields: they can include paths/account identities.
pub(crate) fn control_output(cmd: Command) -> Result<(Option<i32>, Vec<u8>)> {
    control_output_inner(cmd, None, None, Some(Duration::from_secs(30)))
}
fn control_output_inner(
    mut cmd: Command,
    logs: Option<(File, File)>,
    abort: Option<&AtomicBool>,
    timeout: Option<Duration>,
) -> Result<(Option<i32>, Vec<u8>)> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if logs.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    separate_group(&mut cmd);
    let mut child = ChildGuard(cmd.spawn().context("backend executable unavailable")?);
    let (stdout_log, stderr_thread) = if let Some((out, err)) = logs {
        let pipe = child.0.stderr.take().context("missing setup stderr")?;
        (
            Some(out),
            Some(thread::spawn(move || drain(pipe, err, 65536, None))),
        )
    } else {
        (None, None)
    };
    let mut stdout = child.0.stdout.take().context("missing control stdout")?;
    let output = thread::spawn(move || {
        let mut data = Vec::new();
        let mut buf = [0; 8192];
        loop {
            let count = stdout.read(&mut buf)?;
            if count == 0 {
                break;
            }
            // Keep draining after the limit so a verbose subprocess cannot
            // block forever on a full pipe. Retain one extra byte as evidence.
            let keep = count.min((4 * 1024 * 1024 + 1usize).saturating_sub(data.len()));
            data.extend_from_slice(&buf[..keep]);
        }
        if let Some(mut file) = stdout_log {
            file.write_all(&data[..data.len().min(65536)])?;
            file.sync_all()?;
        }
        Ok::<_, std::io::Error>(data)
    });
    let started = Instant::now();
    let mut interrupted = None;
    loop {
        let status = child.0.try_wait()?;
        let complete = status.is_some()
            && output.is_finished()
            && stderr_thread
                .as_ref()
                .is_none_or(|reader| reader.is_finished());
        if abort.is_some_and(|a| a.load(Ordering::SeqCst)) {
            interrupted = Some("backend operation aborted; owned guest cleanup will still run");
        } else if timeout.is_some_and(|timeout| started.elapsed() > timeout) {
            interrupted = Some(
                "backend administrative command exceeded 30 seconds; this helper does not time the agent request",
            );
        }
        if interrupted.is_some() {
            // Kill the whole process group, including descendants retaining pipes
            // after the original transport process has exited.
            kill(&mut child.0);
        }
        if complete || interrupted.is_some() {
            if let Some(reader) = stderr_thread {
                ensure!(
                    !reader
                        .join()
                        .map_err(|_| anyhow::anyhow!("setup stderr reader failed"))?
                        .io_failed,
                    "could not retain private setup stderr"
                );
            }
            let data = output
                .join()
                .map_err(|_| anyhow::anyhow!("control reader failed"))??;
            if let Some(reason) = interrupted {
                anyhow::bail!(reason);
            }
            ensure!(data.len() <= 4 * 1024 * 1024, "oversized backend response");
            return Ok((status.and_then(|status| status.code()), data));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn interactive(mut cmd: Command) -> Result<Option<i32>> {
    // Keep the terminal's process group: the backend owns interactive signal forwarding.
    let mut child = ChildGuard(
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    Ok(child.0.wait()?.code())
}

/// Interactive playback remains cancellable even if the transport does not exit.
pub fn interactive_abort(mut cmd: Command, abort: &AtomicBool) -> Result<Option<i32>> {
    let mut child = ChildGuard(
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    loop {
        if abort.load(Ordering::SeqCst) {
            let _ = child.0.kill();
            return Ok(child.0.wait()?.code());
        }
        if let Some(status) = child.0.try_wait()? {
            return Ok(status.code());
        }
        thread::sleep(Duration::from_millis(50));
    }
}
