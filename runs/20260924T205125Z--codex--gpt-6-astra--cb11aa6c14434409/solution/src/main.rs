mod game;
mod render;
use std::{
    fs, io::{self, IsTerminal, Write},
    path::PathBuf, time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use crossterm::{
    cursor::{Hide, Show, MoveTo},
    event::{
        self, Event, KeyCode, KeyEventKind, KeyModifiers, EnableFocusChange,
        DisableFocusChange, KeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        self, EnterAlternateScreen, LeaveAlternateScreen, DisableLineWrap,
        EnableLineWrap, Clear, ClearType,
    },
    style::{Color, SetForegroundColor, SetBackgroundColor, ResetColor, Print},
};
use game::{Game, Input, Mode};
struct Terminal {
    enhanced: bool,
}
impl Terminal {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut guard = Self { enhanced: false };
        execute!(
            io::stdout(), EnterAlternateScreen, Hide, DisableLineWrap, EnableFocusChange
        )?;
        guard.enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false);
        if guard.enhanced {
            execute!(
                io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
            )?;
        }
        Ok(guard)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        restore(self.enhanced);
    }
}
fn restore(enhanced: bool) {
    if enhanced {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        io::stdout(), DisableFocusChange, EnableLineWrap, Show, ResetColor,
        LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
}
struct Steering {
    until: [Option<Instant>; 4],
    enhanced: bool,
}
impl Steering {
    fn new(enhanced: bool) -> Self {
        Self { until: [None; 4], enhanced }
    }
    fn clear(&mut self) {
        self.until = [None; 4];
    }
    fn key(&mut self, code: KeyCode, kind: KeyEventKind, now: Instant) -> bool {
        let index = match code {
            KeyCode::Left | KeyCode::Char('a') => 0,
            KeyCode::Right | KeyCode::Char('d') => 1,
            KeyCode::Up | KeyCode::Char('w') => 2,
            KeyCode::Down | KeyCode::Char('s') => 3,
            _ => return false,
        };
        if kind == KeyEventKind::Release {
            self.until[index] = None;
        } else {
            self.until[index] = Some(
                now
                    + if self.enhanced {
                        Duration::from_secs(86400)
                    } else {
                        Duration::from_millis(155)
                    },
            );
            if !self.enhanced {
                self.until[index ^ 1] = None;
            }
        }
        true
    }
    fn input(&self, now: Instant) -> Input {
        let active = |i: usize| {
            if self.until[i].is_some_and(|t| t > now) { 1. } else { 0. }
        };
        Input {
            x: active(1) - active(0),
            y: active(3) - active(2),
        }
    }
}
fn score_path() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state"))
        })
        .map(|p| p.join("vesper/best"))
}
fn load_score() -> u32 {
    score_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}
fn save_score(score: u32) {
    if let Some(path) = score_path() {
        if let Some(parent) = path.parent() {
            if fs::create_dir_all(parent).is_ok() {
                let temp = path.with_extension(format!("{}.tmp", std::process::id()));
                if fs::write(&temp, format!("{}\n", score.max(load_score()))).is_ok() {
                    let _ = fs::rename(temp, path);
                }
            }
        }
    }
}
fn run() -> io::Result<()> {
    let terminal = Terminal::enter()?;
    let mut out = io::stdout();
    let mut renderer = render::Renderer::new();
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut g = Game::new(seed, load_score());
    let mut steering = Steering::new(terminal.enhanced);
    let mut size = terminal::size()?;
    let mut previous = Instant::now();
    let mut next_render = previous;
    let mut accumulator = 0.;
    let mut small_shown = false;
    let mut saved = g.best;
    'flight: loop {
        let now = Instant::now();
        let elapsed = now.duration_since(previous).as_secs_f32().min(0.1);
        previous = now;
        let fits = size.0 >= render::WIDTH as u16 && size.1 >= render::HEIGHT as u16;
        for _ in 0..128 {
            if !event::poll(Duration::ZERO)? {
                break;
            }
            match event::read()? {
                Event::Key(k) => {
                    if k.modifiers.contains(KeyModifiers::CONTROL)
                        && k.code == KeyCode::Char('c')
                    {
                        break 'flight;
                    }
                    let code = match k.code {
                        KeyCode::Char(ch) => KeyCode::Char(ch.to_ascii_lowercase()),
                        other => other,
                    };
                    if !fits {
                        if code == KeyCode::Char('q') {
                            break 'flight;
                        }
                        continue;
                    }
                    if g.mode == Mode::Playing && steering.key(code, k.kind, now) {
                        continue;
                    }
                    if k.kind == KeyEventKind::Release || k.kind == KeyEventKind::Repeat
                    {
                        continue;
                    }
                    let before = g.mode;
                    match g.mode {
                        Mode::Title => {
                            match code {
                                KeyCode::Enter => g.start(),
                                KeyCode::Char('q') | KeyCode::Esc => break 'flight,
                                _ => {}
                            }
                        }
                        Mode::Playing => {
                            match code {
                                KeyCode::Char(' ') => g.phase_drive(),
                                KeyCode::Char('x') => g.discharge(),
                                KeyCode::Char('p') | KeyCode::Esc | KeyCode::Char('q') => {
                                    g.mode = Mode::Paused;
                                }
                                _ => {}
                            }
                        }
                        Mode::Paused => {
                            match code {
                                KeyCode::Char('p') | KeyCode::Esc | KeyCode::Enter => {
                                    g.mode = Mode::Playing;
                                }
                                KeyCode::Char('q') => break 'flight,
                                _ => {}
                            }
                        }
                        Mode::Upgrade => {
                            match code {
                                KeyCode::Char('1') => g.upgrade(1),
                                KeyCode::Char('2') => g.upgrade(2),
                                KeyCode::Char('3') => g.upgrade(3),
                                KeyCode::Char('q') => break 'flight,
                                _ => {}
                            }
                        }
                        Mode::Lost | Mode::Won => {
                            match code {
                                KeyCode::Enter | KeyCode::Char('r') => g.start(),
                                KeyCode::Char('q') | KeyCode::Esc => break 'flight,
                                _ => {}
                            }
                        }
                    }
                    if g.mode != before {
                        steering.clear();
                        accumulator = 0.;
                    }
                }
                Event::Resize(w, h) => {
                    size = (w, h);
                    renderer.invalidate();
                    small_shown = false;
                    steering.clear();
                    if g.mode == Mode::Playing {
                        g.mode = Mode::Paused;
                    }
                }
                Event::FocusLost => {
                    steering.clear();
                    if g.mode == Mode::Playing {
                        g.mode = Mode::Paused;
                    }
                }
                _ => {}
            }
        }
        let fits = size.0 >= render::WIDTH as u16 && size.1 >= render::HEIGHT as u16;
        if fits {
            if small_shown {
                small_shown = false;
                renderer.invalidate();
            }
            accumulator += elapsed;
            while accumulator >= 1. / 60. {
                let before = g.mode;
                g.tick(1. / 60., steering.input(now));
                accumulator -= 1. / 60.;
                if g.mode != before {
                    steering.clear();
                }
            }
            if now >= next_render {
                renderer.draw(&mut out, &render::compose(&g), size)?;
                next_render = now + Duration::from_millis(33);
            }
        } else {
            accumulator = 0.;
            if g.mode == Mode::Playing {
                g.mode = Mode::Paused;
            }
            if !small_shown {
                execute!(
                    out, SetBackgroundColor(Color::AnsiValue(233)),
                    SetForegroundColor(Color::AnsiValue(117)), Clear(ClearType::All),
                    MoveTo(0, 0), Print("VESPER / flight suspended")
                )?;
                let message = format!(
                    "Resize to 120 x 65 or larger (now {} x {}). Q exits.", size.0, size
                    .1
                );
                if size.1 > 2 {
                    execute!(
                        out, MoveTo(0, 2), Print(message.chars().take(size.0 as usize)
                        .collect::< String > ())
                    )?;
                }
                out.flush()?;
                small_shown = true;
            }
        }
        if g.best > saved {
            save_score(g.best);
            saved = g.best;
        }
        let _ = event::poll(Duration::from_millis(5))?;
    }
    save_score(g.best.max(g.score));
    Ok(())
}
fn main() {
    if std::env::args().any(|s| s == "--help" || s == "-h") {
        println!(
            "VESPER — a terminal space voyage\n\nRun: cargo run --release --locked\nUse a UTF-8 terminal, 120 columns × 65 rows or larger.\n\nEnter: launch / retry\nWASD or arrows: steer (weapons fire automatically)\nSpace: phase drive — brief invulnerability and speed\nX: nova — clear bullets and damage enemies at 100% charge\nP / Esc: pause or resume\n1 / 2 / 3: choose a refit between sectors\nQ: quit from menus; during flight, open pause\nCtrl-C: quit immediately"
        );
        return;
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!(
            "VESPER needs an interactive terminal. Run cargo run --release --locked in a UTF-8 terminal (120 × 65 or larger)."
        );
        std::process::exit(1);
    }
    let original = std::panic::take_hook();
    std::panic::set_hook(
        Box::new(move |info| {
            restore(true);
            original(info);
        }),
    );
    if let Err(e) = run() {
        eprintln!("VESPER: {e}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_steering_expires_and_reverses() {
        let now = Instant::now();
        let mut s = Steering::new(false);
        s.key(KeyCode::Left, KeyEventKind::Press, now);
        assert_eq!(s.input(now).x,- 1.);
        s.key(KeyCode::Right, KeyEventKind::Press, now);
        assert_eq!(s.input(now).x, 1.);
        assert_eq!(s.input(now + Duration::from_millis(160)).x, 0.);
    }
    #[test]
    fn enhanced_steering_holds_until_release() {
        let now = Instant::now();
        let mut s = Steering::new(true);
        s.key(KeyCode::Char('w'), KeyEventKind::Press, now);
        assert_eq!(s.input(now + Duration::from_secs(2)).y,- 1.);
        s.key(KeyCode::Char('w'), KeyEventKind::Release, now);
        assert_eq!(s.input(now).y, 0.);
    }
}
