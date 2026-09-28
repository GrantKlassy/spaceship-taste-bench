//! Terminal control and keyboard decoding, straight on top of libc.
//!
//! The game needs very little from the terminal: raw mode, the window size,
//! and a stream of key events. Doing it here keeps the dependency tree to a
//! single crate and lets the input layer understand the keyboard-enhancement
//! protocol exactly the way the game wants to use it.

use std::io::{self, Read, Write};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Key events
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyCode {
    Char(char),
    Left,
    Right,
    Up,
    Down,
    Enter,
    Esc,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyKind {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub kind: KeyKind,
    pub ctrl: bool,
}

impl KeyEvent {
    fn press(code: KeyCode) -> Self {
        KeyEvent {
            code,
            kind: KeyKind::Press,
            ctrl: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Escape sequence decoding
// ---------------------------------------------------------------------------

/// Incremental decoder for the byte stream coming from the terminal.
///
/// Understands plain keys, the legacy arrow encodings (CSI and SS3), and the
/// keyboard-enhancement `CSI ... u` form that carries press/repeat/release.
/// Anything else it recognises as a well-formed escape sequence is skipped,
/// so device reports and mouse packets cannot desynchronise it.
#[derive(Default)]
pub struct Decoder {
    pending: Vec<u8>,
    /// Number of consecutive polls that ended on a bare ESC. A real Escape
    /// keypress arrives alone; the start of a sequence is always followed by
    /// more bytes in the same burst.
    lone_esc_polls: u32,
}

impl Decoder {
    pub fn new() -> Self {
        Decoder::default()
    }

    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<KeyEvent>) {
        self.pending.extend_from_slice(bytes);
        let mut i = 0usize;
        let buf = std::mem::take(&mut self.pending);

        while i < buf.len() {
            let b = buf[i];
            match b {
                0x1b => match parse_escape(&buf[i..]) {
                    Parsed::Event(ev, used) => {
                        out.push(ev);
                        i += used;
                    }
                    Parsed::Ignored(used) => i += used,
                    Parsed::Incomplete => break,
                },
                b'\r' | b'\n' => {
                    out.push(KeyEvent::press(KeyCode::Enter));
                    i += 1;
                }
                // Control characters: Ctrl-A is 0x01, so add 0x60 to recover
                // the letter. Tab, LF and CR are handled above or ignored.
                0x01..=0x1a => {
                    let ch = (b + 0x60) as char;
                    out.push(KeyEvent {
                        code: KeyCode::Char(ch),
                        kind: KeyKind::Press,
                        ctrl: true,
                    });
                    i += 1;
                }
                0x20..=0x7e => {
                    out.push(KeyEvent::press(KeyCode::Char(b as char)));
                    i += 1;
                }
                // The game binds no non-ASCII keys; drop anything else rather
                // than trying to decode it.
                _ => i += 1,
            }
        }

        self.pending = buf[i..].to_vec();

        // Resolve a trailing bare ESC once it has survived a poll on its own.
        if self.pending == [0x1b] {
            self.lone_esc_polls += 1;
            if self.lone_esc_polls >= 2 {
                out.push(KeyEvent::press(KeyCode::Esc));
                self.pending.clear();
                self.lone_esc_polls = 0;
            }
        } else {
            self.lone_esc_polls = 0;
            // A sequence that never terminates would wedge the decoder, so
            // give up on anything implausibly long.
            if self.pending.len() > 64 {
                self.pending.clear();
            }
        }
    }
}

enum Parsed {
    Event(KeyEvent, usize),
    Ignored(usize),
    Incomplete,
}

/// Parse one escape sequence starting at `b[0] == 0x1b`.
fn parse_escape(b: &[u8]) -> Parsed {
    if b.len() < 2 {
        return Parsed::Incomplete;
    }
    match b[1] {
        b'[' => parse_csi(b),
        // SS3: some terminals send arrows as ESC O A in application mode.
        b'O' => {
            if b.len() < 3 {
                return Parsed::Incomplete;
            }
            match arrow_of(b[2]) {
                Some(code) => Parsed::Event(KeyEvent::press(code), 3),
                None => Parsed::Ignored(3),
            }
        }
        // Alt-modified key; the game has no Alt bindings.
        _ => Parsed::Ignored(2),
    }
}

fn arrow_of(final_byte: u8) -> Option<KeyCode> {
    Some(match final_byte {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        _ => return None,
    })
}

fn parse_csi(b: &[u8]) -> Parsed {
    // CSI = ESC [ ; then parameter bytes 0x30..=0x3f, intermediates
    // 0x20..=0x2f, and a final byte 0x40..=0x7e.
    let mut i = 2usize;
    while i < b.len() && (0x30..=0x3f).contains(&b[i]) {
        i += 1;
    }
    while i < b.len() && (0x20..=0x2f).contains(&b[i]) {
        i += 1;
    }
    if i >= b.len() {
        return Parsed::Incomplete;
    }
    let final_byte = b[i];
    if !(0x40..=0x7e).contains(&final_byte) {
        return Parsed::Ignored(i + 1);
    }
    let used = i + 1;
    let params = &b[2..i];

    // Private sequences (CSI ? ...) are replies, never key presses.
    if params.first() == Some(&b'?') || params.first() == Some(&b'<') {
        return Parsed::Ignored(used);
    }

    let p = Params::parse(params);
    match final_byte {
        b'A' | b'B' | b'C' | b'D' => {
            let Some(code) = arrow_of(final_byte) else {
                return Parsed::Ignored(used);
            };
            // CSI 1 ; mods : event A
            let (ctrl, kind) = p.modifiers(1);
            Parsed::Event(KeyEvent { code, kind, ctrl }, used)
        }
        b'u' => {
            // CSI unicode ; mods : event u
            let Some(cp) = p.get(0, 0) else {
                return Parsed::Ignored(used);
            };
            let (ctrl, kind) = p.modifiers(1);
            let code = match cp {
                13 | 10 => KeyCode::Enter,
                27 => KeyCode::Esc,
                57417 => KeyCode::Left,
                57418 => KeyCode::Right,
                57419 => KeyCode::Up,
                57420 => KeyCode::Down,
                c => match char::from_u32(c) {
                    Some(ch) if !ch.is_control() => KeyCode::Char(ch),
                    _ => return Parsed::Ignored(used),
                },
            };
            Parsed::Event(KeyEvent { code, kind, ctrl }, used)
        }
        // Mouse reports, device attributes, cursor position reports, etc.
        _ => Parsed::Ignored(used),
    }
}

/// Semicolon-separated CSI parameters, each of which may carry
/// colon-separated sub-parameters.
struct Params {
    groups: Vec<Vec<Option<u32>>>,
}

impl Params {
    fn parse(bytes: &[u8]) -> Params {
        let mut groups = Vec::new();
        let mut group: Vec<Option<u32>> = Vec::new();
        let mut cur: Option<u32> = None;
        for &c in bytes {
            match c {
                b'0'..=b'9' => {
                    let d = (c - b'0') as u32;
                    cur = Some(cur.unwrap_or(0).saturating_mul(10).saturating_add(d));
                }
                b':' => {
                    group.push(cur.take());
                }
                b';' => {
                    group.push(cur.take());
                    groups.push(std::mem::take(&mut group));
                }
                _ => {}
            }
        }
        group.push(cur);
        groups.push(group);
        Params { groups }
    }

    fn get(&self, group: usize, sub: usize) -> Option<u32> {
        self.groups.get(group)?.get(sub).copied().flatten()
    }

    /// Decode the modifier group: `mods` is a bitmask plus one, and the
    /// optional sub-parameter is the event type (1 press, 2 repeat, 3 release).
    fn modifiers(&self, group: usize) -> (bool, KeyKind) {
        let mods = self.get(group, 0).unwrap_or(1).saturating_sub(1);
        let ctrl = mods & 0b100 != 0;
        let kind = match self.get(group, 1).unwrap_or(1) {
            2 => KeyKind::Repeat,
            3 => KeyKind::Release,
            _ => KeyKind::Press,
        };
        (ctrl, kind)
    }
}

// ---------------------------------------------------------------------------
// Terminal mode
// ---------------------------------------------------------------------------

struct Saved(libc::termios);
// Written once before any other thread exists and only ever read afterwards.
unsafe impl Send for Saved {}
unsafe impl Sync for Saved {}

static ORIGINAL: OnceLock<Saved> = OnceLock::new();

const ENTER: &str = concat!(
    "\x1b[?1049h", // alternate screen
    "\x1b[?25l",   // hide cursor
    "\x1b[?7l",    // no autowrap, so writing the last column cannot scroll
    "\x1b[>2u",    // keyboard enhancement: report event types
    "\x1b[2J",     // clear
);

const LEAVE: &str = concat!(
    "\x1b[<u", // pop keyboard enhancement
    "\x1b[?7h", "\x1b[?25h", "\x1b[m", "\x1b[?1049l",
);

/// Puts the terminal into raw mode and restores it on drop, including when
/// the process panics.
pub struct Tty {
    decoder: Decoder,
    restored: bool,
}

fn set_termios(t: &libc::termios) -> io::Result<()> {
    // SAFETY: `t` is a valid, fully initialised termios for fd 0.
    let rc = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl Tty {
    pub fn enter() -> io::Result<Tty> {
        // SAFETY: zeroed termios is a valid destination for tcgetattr.
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: fd 0 with a valid out-pointer.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut t) } != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "standard input is not a terminal",
            ));
        }
        let _ = ORIGINAL.set(Saved(t));
        install_signal_handlers();

        let mut raw = t;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::INPCK | libc::ISTRIP);
        raw.c_oflag &= !libc::OPOST;
        // Return from read() immediately with whatever is available, so the
        // frame loop never blocks on input.
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 0;
        set_termios(&raw)?;

        let mut out = io::stdout();
        out.write_all(ENTER.as_bytes())?;
        out.flush()?;

        Ok(Tty {
            decoder: Decoder::new(),
            restored: false,
        })
    }

    /// Current terminal size as (columns, rows).
    pub fn size(&self) -> (usize, usize) {
        // SAFETY: zeroed winsize is a valid destination for TIOCGWINSZ.
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: fd 0 with a valid out-pointer for this ioctl.
        let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
        if rc != 0 || ws.ws_col == 0 || ws.ws_row == 0 {
            (80, 24)
        } else {
            (ws.ws_col as usize, ws.ws_row as usize)
        }
    }

    /// Drain whatever the terminal has sent since the last call.
    ///
    /// VMIN/VTIME are both zero, so `read` returns straight away with whatever
    /// is buffered and this never blocks the frame loop.
    pub fn poll(&mut self, out: &mut Vec<KeyEvent>) {
        let mut buf = [0u8; 4096];
        let mut stdin = io::stdin().lock();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.decoder.feed(&buf[..n], out);
                    if n < buf.len() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        // Give the decoder an empty poll so a bare ESC can time out even when
        // no new bytes arrived.
        self.decoder.feed(&[], out);
    }

    pub fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        restore_global();
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Undo everything `Tty::enter` did. Safe to call more than once, and callable
/// from a panic hook.
pub fn restore_global() {
    let mut out = io::stdout();
    let _ = out.write_all(LEAVE.as_bytes());
    let _ = out.flush();
    if let Some(Saved(t)) = ORIGINAL.get() {
        let _ = set_termios(t);
    }
}

/// Restore the terminal and exit. Only calls async-signal-safe functions, so
/// it is legal to run from a signal handler.
extern "C" fn on_fatal_signal(sig: libc::c_int) {
    // SAFETY: write(2), tcsetattr(3) and _exit(2) are all async-signal-safe.
    // `ORIGINAL` is written before any handler is installed and only read here.
    unsafe {
        libc::write(
            libc::STDOUT_FILENO,
            LEAVE.as_ptr() as *const libc::c_void,
            LEAVE.len(),
        );
        if let Some(Saved(t)) = ORIGINAL.get() {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t);
        }
        libc::_exit(128 + sig);
    }
}

/// Make sure a kill signal cannot leave the terminal in raw mode. Without
/// this, `kill` on a running game leaves the user with no echo and no
/// line editing.
fn install_signal_handlers() {
    for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT] {
        // SAFETY: installing a handler that is async-signal-safe.
        unsafe {
            let handler: extern "C" fn(libc::c_int) = on_fatal_signal;
            libc::signal(sig, handler as *const () as libc::sighandler_t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(s: &[u8]) -> Vec<KeyEvent> {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        d.feed(s, &mut out);
        out
    }

    #[test]
    fn plain_keys_and_enter() {
        let ev = decode(b"wasd\r");
        let codes: Vec<KeyCode> = ev.iter().map(|e| e.code).collect();
        assert_eq!(
            codes,
            vec![
                KeyCode::Char('w'),
                KeyCode::Char('a'),
                KeyCode::Char('s'),
                KeyCode::Char('d'),
                KeyCode::Enter,
            ]
        );
        assert!(ev.iter().all(|e| e.kind == KeyKind::Press));
    }

    #[test]
    fn legacy_arrows_in_both_encodings() {
        for (seq, want) in [
            (&b"\x1b[A"[..], KeyCode::Up),
            (&b"\x1b[B"[..], KeyCode::Down),
            (&b"\x1b[C"[..], KeyCode::Right),
            (&b"\x1b[D"[..], KeyCode::Left),
            (&b"\x1bOA"[..], KeyCode::Up),
            (&b"\x1bOD"[..], KeyCode::Left),
        ] {
            let ev = decode(seq);
            assert_eq!(ev.len(), 1, "{seq:?}");
            assert_eq!(ev[0].code, want, "{seq:?}");
            assert_eq!(ev[0].kind, KeyKind::Press);
        }
    }

    #[test]
    fn ctrl_c_is_recognised() {
        let ev = decode(b"\x03");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].code, KeyCode::Char('c'));
        assert!(ev[0].ctrl);
    }

    #[test]
    fn enhanced_key_events_carry_press_repeat_and_release() {
        // CSI 97 ; 1 : <event> u  -> 'a'
        for (ev_num, want) in [(1u8, KeyKind::Press), (2, KeyKind::Repeat), (3, KeyKind::Release)] {
            let seq = format!("\x1b[97;1:{ev_num}u");
            let ev = decode(seq.as_bytes());
            assert_eq!(ev.len(), 1, "{seq:?}");
            assert_eq!(ev[0].code, KeyCode::Char('a'));
            assert_eq!(ev[0].kind, want, "{seq:?}");
        }
    }

    #[test]
    fn enhanced_arrows_carry_release() {
        let ev = decode(b"\x1b[1;1:3D");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].code, KeyCode::Left);
        assert_eq!(ev[0].kind, KeyKind::Release);
    }

    #[test]
    fn enhanced_form_without_subparams() {
        let ev = decode(b"\x1b[32u");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].code, KeyCode::Char(' '));
        assert_eq!(ev[0].kind, KeyKind::Press);
    }

    #[test]
    fn ctrl_modifier_is_decoded() {
        // mods = 5 -> bitmask 4 -> ctrl
        let ev = decode(b"\x1b[99;5u");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].code, KeyCode::Char('c'));
        assert!(ev[0].ctrl);
    }

    #[test]
    fn device_reports_are_skipped_without_desync() {
        // A DA1 reply and a cursor position report bracketing a real key.
        let ev = decode(b"\x1b[?62;c\x1b[24;80R\x1b[Ax");
        let codes: Vec<KeyCode> = ev.iter().map(|e| e.code).collect();
        assert_eq!(codes, vec![KeyCode::Up, KeyCode::Char('x')]);
    }

    #[test]
    fn sequences_split_across_reads_are_reassembled() {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        d.feed(b"\x1b", &mut out);
        assert!(out.is_empty(), "must wait for the rest of the sequence");
        d.feed(b"[", &mut out);
        assert!(out.is_empty());
        d.feed(b"1;1:3", &mut out);
        assert!(out.is_empty());
        d.feed(b"C", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, KeyCode::Right);
        assert_eq!(out[0].kind, KeyKind::Release);
    }

    #[test]
    fn a_bare_escape_resolves_after_an_idle_poll() {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        d.feed(b"\x1b", &mut out);
        assert!(out.is_empty(), "could still be the start of a sequence");
        d.feed(&[], &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, KeyCode::Esc);
        // And it does not fire twice.
        d.feed(&[], &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn escape_followed_by_a_sequence_is_not_read_as_escape() {
        let ev = decode(b"\x1b[D");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].code, KeyCode::Left);
    }

    #[test]
    fn a_runaway_sequence_cannot_wedge_the_decoder() {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        d.feed(b"\x1b[", &mut out);
        d.feed(&vec![b'1'; 200], &mut out);
        assert!(out.is_empty());
        // The decoder has dropped the junk and still reads the next key.
        d.feed(b"q", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, KeyCode::Char('q'));
    }

    #[test]
    fn kitty_functional_arrow_codepoints() {
        let ev = decode(b"\x1b[57417;1:1u");
        assert_eq!(ev[0].code, KeyCode::Left);
    }
}
