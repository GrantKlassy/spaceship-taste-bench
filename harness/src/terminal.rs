use anyhow::{Result, bail, ensure};

#[cfg(unix)]
pub struct Terminal {
    saved: libc::termios,
    input: libc::c_int,
    output: libc::c_int,
}
#[cfg(unix)]
impl Terminal {
    pub fn require_size(columns: u16, rows: u16) -> Result<Self> {
        Self::capture(0, 1, columns, rows)
    }
    // Private: these descriptors must stay open until the guard is dropped.
    fn capture(input: libc::c_int, output: libc::c_int, columns: u16, rows: u16) -> Result<Self> {
        // SAFETY: stack structs have valid storage, fd 0 is checked for a TTY.
        unsafe {
            ensure!(
                libc::isatty(input) == 1 && libc::isatty(output) == 1,
                "play requires an interactive terminal sized to {columns} columns by {rows} rows"
            );
            let mut size: libc::winsize = std::mem::zeroed();
            ensure!(
                libc::ioctl(input, libc::TIOCGWINSZ, &mut size) == 0,
                "cannot query terminal dimensions"
            );
            ensure!(
                size.ws_col == columns && size.ws_row == rows,
                "resize terminal to {columns}x{rows} (currently {}x{})",
                size.ws_col,
                size.ws_row
            );
            let mut saved: libc::termios = std::mem::zeroed();
            ensure!(
                libc::tcgetattr(input, &mut saved) == 0,
                "cannot save terminal state"
            );
            Ok(Self {
                saved,
                input,
                output,
            })
        }
    }
}
#[cfg(unix)]
impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: saved was obtained from tcgetattr on this terminal.
        unsafe {
            libc::tcsetattr(self.input, libc::TCSANOW, &self.saved);
            let reset = b"\x1b[0m\x1b[?25h\x1b[?1049l\x1b[?2004l";
            libc::write(self.output, reset.as_ptr().cast(), reset.len());
        }
    }
}
#[cfg(not(unix))]
pub struct Terminal;
#[cfg(not(unix))]
impl Terminal {
    pub fn require_size(_columns: u16, _rows: u16) -> Result<Self> {
        bail!("native Windows terminal transport is unsupported")
    }
}

pub fn supported_host() -> Result<()> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        bail!(
            "this resolved environment requires Ubuntu 24.04+ amd64 with Linux-local sbx/KVM; macOS, native Windows and other architectures are not enabled in v1"
        )
    }
    let os = std::fs::read_to_string("/etc/os-release")?;
    ensure!(
        supported_linux(&os),
        "this backend integration is enabled only for Ubuntu 24.04+; use an Ubuntu host or WSL2 Ubuntu with working KVM. Fedora/other distributions still need backend verification"
    );
    ensure!(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .is_ok(),
        "local sbx requires read/write access to /dev/kvm; see docs/SETUP.md for WSL nested virtualization and group setup"
    );
    Ok(())
}

fn supported_linux(os: &str) -> bool {
    let field = |key: &str| {
        os.lines()
            .find_map(|line| line.strip_prefix(key))
            .map(|v| v.trim_matches('"'))
    };
    field("ID=") == Some("ubuntu")
        && field("VERSION_ID=")
            .and_then(|v| {
                let (major, minor) = v.split_once('.')?;
                Some((major.parse::<u16>().ok()?, minor.parse::<u16>().ok()?))
            })
            .is_some_and(|version| version >= (24, 4))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_distro_is_checked_independently_of_rust_host_support() {
        assert!(supported_linux("ID=ubuntu\nVERSION_ID=\"24.04\"\n"));
        assert!(supported_linux("ID=ubuntu\nVERSION_ID=\"26.04\"\n"));
        for os in [
            "ID=fedora\nVERSION_ID=44\n",
            "ID=ubuntu\nVERSION_ID=22.04\n",
            "ID=ubuntu\n",
            "ID=debian\nVERSION_ID=13\n",
        ] {
            assert!(!supported_linux(os));
        }
    }
    #[cfg(unix)]
    #[test]
    fn terminal_guard_verifies_real_pty_dimensions_and_restores_modes() {
        use std::os::fd::{FromRawFd, OwnedFd};
        // SAFETY: openpty writes initialized descriptors; owned handles outlive
        // every guard and termios struct is filled before it is read.
        unsafe {
            let mut master = -1;
            let mut slave = -1;
            let mut size = libc::winsize {
                ws_row: 40,
                ws_col: 120,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            assert_eq!(
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    &size
                ),
                0
            );
            let _master = OwnedFd::from_raw_fd(master);
            let _slave = OwnedFd::from_raw_fd(slave);
            drop(Terminal::capture(slave, slave, 120, 40).unwrap());
            assert!(Terminal::capture(slave, slave, 124, 69).is_err());
            size.ws_col = 124;
            size.ws_row = 69;
            assert_eq!(libc::ioctl(slave, libc::TIOCSWINSZ, &size), 0);
            assert!(Terminal::capture(slave, slave, 120, 40).is_err());
            let guard = Terminal::capture(slave, slave, 124, 69).unwrap();
            let saved = guard.saved;
            let mut raw = saved;
            libc::cfmakeraw(&mut raw);
            assert_eq!(libc::tcsetattr(slave, libc::TCSANOW, &raw), 0);
            drop(guard);
            let mut restored: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(slave, &mut restored), 0);
            assert_eq!(
                (
                    restored.c_iflag,
                    restored.c_oflag,
                    restored.c_cflag,
                    restored.c_lflag,
                    restored.c_cc
                ),
                (
                    saved.c_iflag,
                    saved.c_oflag,
                    saved.c_cflag,
                    saved.c_lflag,
                    saved.c_cc
                )
            );
            size.ws_col = 80;
            assert_eq!(libc::ioctl(slave, libc::TIOCSWINSZ, &size), 0);
            assert!(Terminal::capture(slave, slave, 124, 69).is_err());
        }
    }
}
