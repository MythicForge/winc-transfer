// Prevents an extra console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod crypto;
mod import;
mod logbus;
mod model;
mod net;
mod settings;
mod sources;
mod transfer;
mod tui;
mod ui;

use std::io::IsTerminal;
use std::process::ExitCode;

/// Folder name under %APPDATA% / %LOCALAPPDATA% — the same one the Tauri
/// builds used, so settings and logs carry over.
pub const APP_ID: &str = "com.winc.app";

const HELP: &str = "\
Usage: winci [--tui | --gui]

  --tui          terminal UI, keyboard + mouse: for servers and SSH sessions
  --gui          the window, even over SSH (X forwarding)
  -V, --version  print the version
  -h, --help     print this help

With neither flag, WINCI opens the terminal UI when it's started from a
terminal over SSH or with no graphical display (DISPLAY / WAYLAND_DISPLAY
unset), and the window otherwise.

NO_COLOR=1 drops colours; a non-UTF-8 locale switches to plain ASCII.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Gui,
    Tui,
}

#[derive(PartialEq, Eq, Debug)]
enum Cli {
    /// the mode, and whether a flag chose it
    Run(Mode, bool),
    Help,
    Version,
    Bad(String),
}

/// Which UI to open. `terminal`: stdin and stdout are both a terminal.
fn parse_cli(args: &[String], env: impl Fn(&str) -> Option<String>, terminal: bool) -> Cli {
    let mut forced = None;
    for a in args {
        match a.as_str() {
            "--tui" => forced = Some(Mode::Tui),
            "--gui" => forced = Some(Mode::Gui),
            "-h" | "--help" => return Cli::Help,
            "-V" | "--version" => return Cli::Version,
            other => return Cli::Bad(other.to_string()),
        }
    }
    match forced {
        Some(m) => Cli::Run(m, true),
        None => Cli::Run(auto_mode(env, terminal), false),
    }
}

/// No flag: the terminal UI over SSH or with no display to draw a window on
/// (Linux/BSD only — Windows and macOS always have one), the window otherwise.
fn auto_mode(env: impl Fn(&str) -> Option<String>, terminal: bool) -> Mode {
    if !cfg!(all(unix, not(target_os = "macos"))) || !terminal {
        return Mode::Gui;
    }
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    let ssh = set("SSH_CONNECTION") || set("SSH_TTY");
    let display = set("DISPLAY") || set("WAYLAND_DISPLAY");
    if ssh || !display {
        Mode::Tui
    } else {
        Mode::Gui
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let (mode, forced) = match parse_cli(&args, |k| std::env::var(k).ok(), terminal) {
        Cli::Run(m, forced) => (m, forced),
        Cli::Help => {
            println!("WINCI {} — PC-to-PC data crossing\n\n{HELP}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Cli::Version => {
            println!("winci {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Cli::Bad(a) => {
            eprintln!("winci: unknown option '{a}'\n\n{HELP}");
            return ExitCode::from(2);
        }
    };

    // Always on, release included: the log view and the on-disk log are the
    // only way a fault on someone else's PC can be diagnosed.
    logbus::init();
    log::info!("ui: {mode:?} ({})", if forced { "flag" } else { "auto" });
    match mode {
        Mode::Tui if !terminal => {
            if cfg!(windows) {
                // release builds have no console to draw in
                log::warn!("--tui needs a console, and this build has none — opening the window instead");
                gui()
            } else {
                log::error!("--tui needs an interactive terminal (stdin and stdout)");
                eprintln!("winci: --tui needs an interactive terminal (stdin and stdout)");
                ExitCode::from(2)
            }
        }
        Mode::Tui => match tui::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                log::error!("terminal UI failed: {e}");
                eprintln!("winci: terminal UI failed: {e}");
                ExitCode::FAILURE
            }
        },
        Mode::Gui => gui(),
    }
}

fn gui() -> ExitCode {
    match ui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("the window failed to start: {e}");
            eprintln!("winci: the window failed to start: {e}\n(over SSH or on a server, try: winci --tui)");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }
    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_win() {
        let desk = env(&[("DISPLAY", ":0")]);
        assert_eq!(parse_cli(&args(&["--tui"]), &desk, true), Cli::Run(Mode::Tui, true));
        assert_eq!(parse_cli(&args(&["--gui"]), env(&[("SSH_TTY", "/dev/pts/1")]), true), Cli::Run(Mode::Gui, true));
        assert_eq!(parse_cli(&args(&["--tui"]), &desk, false), Cli::Run(Mode::Tui, true));
        assert_eq!(parse_cli(&args(&["-h"]), &desk, true), Cli::Help);
        assert_eq!(parse_cli(&args(&["--version"]), &desk, true), Cli::Version);
        assert_eq!(parse_cli(&args(&["--bogus"]), &desk, true), Cli::Bad("--bogus".into()));
    }

    #[test]
    fn auto_detection() {
        if !cfg!(all(unix, not(target_os = "macos"))) {
            // always the window there
            assert_eq!(parse_cli(&[], env(&[]), true), Cli::Run(Mode::Gui, false));
            return;
        }
        let run = |e, terminal| parse_cli(&[], e, terminal);
        // a desktop terminal: the window
        assert_eq!(run(env(&[("DISPLAY", ":0")]), true), Cli::Run(Mode::Gui, false));
        assert_eq!(run(env(&[("WAYLAND_DISPLAY", "wayland-0")]), true), Cli::Run(Mode::Gui, false));
        // SSH, even with X forwarding: the terminal UI
        assert_eq!(run(env(&[("SSH_CONNECTION", "10.0.0.2 5022 10.0.0.9 22")]), true), Cli::Run(Mode::Tui, false));
        assert_eq!(run(env(&[("SSH_TTY", "/dev/pts/3"), ("DISPLAY", "localhost:10.0")]), true), Cli::Run(Mode::Tui, false));
        // a server console with no display
        assert_eq!(run(env(&[]), true), Cli::Run(Mode::Tui, false));
        assert_eq!(run(env(&[("DISPLAY", "")]), true), Cli::Run(Mode::Tui, false));
        // no terminal (a desktop launcher, a pipe): the window
        assert_eq!(run(env(&[("SSH_TTY", "/dev/pts/3")]), false), Cli::Run(Mode::Gui, false));
    }
}
