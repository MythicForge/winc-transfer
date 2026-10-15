//! Terminal UI (`winci --tui`, or automatic over SSH / with no display): the
//! whole send / receive flow in a terminal, driven by keyboard or mouse. Same
//! backend, same wire protocol, so it pairs with the window on the other PC.
//!
//! Built on ratatui + crossterm. Layout and hit-testing live in `kit`, the
//! step flow in `app`, the screens in `screens`, the frame in `view`.

mod app;
mod input;
mod kit;
mod picker;
mod screens;
mod view;

use crate::backend;
use app::{Jobs, Msg, TuiApp};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, MouseEventKind};
use ratatui::crossterm::{execute, terminal};
use ratatui::Terminal;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

/// The thread drawing the UI: only a panic there tears the terminal down.
static UI_THREAD: OnceLock<ThreadId> = OnceLock::new();
/// A worker thread panicked: repaint the whole screen (the default hook
/// would have scribbled over it).
static REPAINT: AtomicBool = AtomicBool::new(false);

/// How often the "..." dots step.
const TICK: Duration = Duration::from_millis(400);

/// UTF-8 locale? Decides box-drawing vs plain-ASCII glyphs. The first of
/// LC_ALL / LC_CTYPE / LANG that is set wins, as in libc.
fn utf8_locale(env: impl Fn(&str) -> Option<String>) -> bool {
    if cfg!(windows) {
        return true;
    }
    for k in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Some(v) = env(k).filter(|v| !v.is_empty()) {
            let v = v.to_lowercase();
            return v.contains("utf-8") || v.contains("utf8");
        }
    }
    false
}

/// Clicks and the wheel only (X10 + SGR encoding). crossterm's
/// EnableMouseCapture also turns on any-motion reporting, which floods an SSH
/// link with an event per cell the pointer crosses.
#[cfg(unix)]
fn mouse_on() -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(b"\x1b[?1000h\x1b[?1006h")?;
    out.flush()
}
#[cfg(unix)]
fn mouse_off() {
    let mut out = io::stdout();
    let _ = out.write_all(b"\x1b[?1006l\x1b[?1000l");
    let _ = out.flush();
}
#[cfg(not(unix))]
fn mouse_on() -> io::Result<()> {
    execute!(io::stdout(), event::EnableMouseCapture)
}
#[cfg(not(unix))]
fn mouse_off() {
    let _ = execute!(io::stdout(), event::DisableMouseCapture);
}

/// Put the terminal back the way the shell had it. Safe to call twice.
fn restore() {
    mouse_off();
    let _ = execute!(
        io::stdout(),
        event::DisableBracketedPaste,
        terminal::LeaveAlternateScreen,
        ratatui::crossterm::cursor::Show
    );
    let _ = terminal::disable_raw_mode();
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if UI_THREAD.get() == Some(&std::thread::current().id()) {
            // the UI is going down: give the shell its terminal back first,
            // then let logbus's hook record it and print it where it can be read
            restore();
            prev(info);
        } else {
            // a worker died (the window's logbus hook logs these too); the UI
            // carries on, so keep stderr off the screen and repaint it
            crate::logbus::record("error", "winci::panic", format!("PANIC: {info}"));
            REPAINT.store(true, Ordering::Relaxed);
        }
    }));
}

/// Copy to the clipboard of the terminal the user is sitting at — over SSH
/// too — with an OSC 52 escape. Most modern terminals honour it; tmux needs
/// `set -g set-clipboard on`.
pub(crate) fn osc52_copy(text: &str) {
    // terminals cap the sequence; keep the newest part of the log
    const MAX: usize = 74_000;
    let bytes = text.as_bytes();
    let bytes = &bytes[bytes.len().saturating_sub(MAX)..];
    let b64 = base64(bytes);
    let seq = if std::env::var_os("TMUX").is_some() {
        format!("\x1bPtmux;\x1b\x1b]52;c;{b64}\x07\x1b\\")
    } else {
        format!("\x1b]52;c;{b64}\x07")
    };
    let mut out = io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn run() -> io::Result<()> {
    let env = |k: &str| std::env::var(k).ok();
    let utf8 = utf8_locale(env);
    kit::set_ascii(!utf8);
    kit::set_no_color(env("NO_COLOR").is_some_and(|v| !v.is_empty()));
    let _ = UI_THREAD.set(std::thread::current().id());

    terminal::enable_raw_mode()?;
    install_panic_hook();
    let setup = execute!(io::stdout(), terminal::EnterAlternateScreen, event::EnableBracketedPaste)
        .and_then(|_| mouse_on());
    let out = setup
        .and_then(|_| Terminal::new(CrosstermBackend::new(io::stdout())))
        .and_then(|mut term| event_loop(&mut term, utf8));
    restore();
    out
}

fn event_loop(term: &mut Terminal<CrosstermBackend<io::Stdout>>, utf8: bool) -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut app = TuiApp::new(Jobs(tx));
    let mut backend_rx = backend::take_events();

    // the TUI's version of the window's "renderer:" line
    let size = term.size()?;
    let ssh = std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
    log::info!(
        "ui: terminal TERM={} {}×{} · ssh={} · {}",
        std::env::var("TERM").unwrap_or_else(|_| "?".into()),
        size.width,
        size.height,
        if ssh { "yes" } else { "no" },
        if utf8 { "utf-8" } else { "ascii glyphs (locale isn't UTF-8)" }
    );

    let mut dirty = true;
    let mut last_tick = Instant::now();
    loop {
        if REPAINT.swap(false, Ordering::Relaxed) {
            term.clear()?;
            dirty = true;
        }
        if dirty {
            term.draw(|f| view::draw(f, &mut app))?;
            dirty = false;
        }
        if app.quit {
            break;
        }

        if event::poll(Duration::from_millis(50))? {
            // drain whatever arrived, then draw once
            loop {
                match event::read()? {
                    Event::Key(k) => {
                        app.key(k);
                        dirty = true;
                    }
                    Event::Mouse(m) => {
                        if matches!(
                            m.kind,
                            MouseEventKind::Down(_) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                        ) {
                            app.mouse(m);
                            dirty = true;
                        }
                    }
                    Event::Paste(s) => {
                        app.paste(&s);
                        dirty = true;
                    }
                    Event::Resize(..) => dirty = true,
                    _ => {}
                }
                if app.quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
        while let Ok(m) = rx.try_recv() {
            app.update(m);
            dirty = true;
        }
        if let Some(r) = backend_rx.as_mut() {
            while let Ok(ev) = r.try_recv() {
                app.update(Msg::Backend(ev));
                dirty = true;
            }
        }
        if last_tick.elapsed() >= TICK {
            app.tick += 1;
            last_tick = Instant::now();
            dirty = true;
        }
        if app.timers(Instant::now()) {
            dirty = true;
        }
    }

    // stop anything still in flight: beacon, listener, transfer
    if app.role.is_some() {
        backend::cancel();
    }
    log::info!("ui: terminal UI closed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_detection() {
        if cfg!(windows) {
            return;
        }
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert!(utf8_locale(env(&[("LANG", "en_US.UTF-8")])));
        assert!(utf8_locale(env(&[("LC_ALL", "C.utf8"), ("LANG", "C")])));
        // the first one set wins
        assert!(!utf8_locale(env(&[("LC_ALL", "C"), ("LANG", "en_US.UTF-8")])));
        assert!(!utf8_locale(env(&[])));
        assert!(utf8_locale(env(&[("LC_ALL", ""), ("LANG", "de_DE.UTF-8")])));
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
