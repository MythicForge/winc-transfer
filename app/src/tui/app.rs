//! TUI state + messages + update: the same step flow as the window
//! (`ui/mod.rs`), with blocking backend work run on threads (`Jobs`) instead
//! of iced tasks, and terminal key / mouse events routed through the hit map.

use super::input::LineEdit;
use super::kit::{Hit, Kind, Region};
use super::picker::{PickFor, PickKind, Picker};
use crate::backend::{self, DropSettings, Event, ReceiverInfo};
use crate::import::{ImportEntry, ImportReport};
use crate::logbus::{self, LogEntry};
use crate::model::{AdapterInfo, LinkStatus, Peer, SourceGroup, TransferProgress};
use crate::ui::format;
use crate::ui::{
    idle_progress, merge_picks, pair_error_text, parse_addr, BrowserAction, Fw, PairMode, PairStatus, PickInto,
    Role, Step, TransferMode,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const IN_CODE: &str = "in:code";
pub const IN_ADDR: &str = "in:addr";
pub const IN_FILTER: &str = "in:filter";
pub const IN_PATH: &str = "in:path";
pub const PK_LIST: &str = "pk:list";

/// Picker actions from its buttons and rows.
#[derive(Debug, Clone, PartialEq)]
pub enum Pk {
    Click(usize),
    Mark(usize),
    Parent,
    Home,
    Hidden,
    Confirm,
    Cancel,
    PathSubmit,
}

#[derive(Debug, Clone)]
pub enum Msg {
    // shell
    SetRole(Role),
    Reset,
    GoTo(Step),
    ToggleLogs,
    Quit,
    /// the "Stop and quit?" box: true = quit
    QuitAnswer(bool),
    Backend(Event),
    // connect
    PollLink,
    Link(LinkStatus),
    PollAdapters,
    Adapters(Vec<AdapterInfo>),
    AllowFirewall,
    Firewall(Result<(), String>),
    // pair (sender)
    SetPairMode(PairMode),
    Discovered(u32, Option<Peer>),
    AddrSubmit,
    SubmitPair,
    Paired(u64, Result<Vec<SourceGroup>, String>),
    // code (receiver)
    ReceiverStarted(u64, Result<ReceiverInfo, String>),
    LocalIps(Vec<String>),
    Drop(DropSettings),
    DropFree(Option<u64>),
    ChangeDrop,
    ResetDrop,
    DropSet(Result<DropSettings, String>),
    Received(u64, Result<String, String>),
    // select (sender)
    SetMode(TransferMode),
    Toggle(String),
    SelectAll(bool),
    Pick(PickInto),
    Picked(PickInto, Option<Vec<SourceGroup>>),
    RemovePick(String),
    ClearPicks,
    StartTransfer,
    RunBench,
    Bench(u64, Result<f64, String>),
    // transfer
    Stop,
    Sent(u64, Result<(), String>),
    // done (receiver import)
    UseImportFolder(bool),
    ChooseImportFolder,
    ImportFolder(Option<String>),
    ImportFree(Option<u64>),
    RunImport,
    Imported(Result<ImportReport, String>),
    Browser(String, BrowserAction),
    BrowserDone(String, Result<ImportEntry, String>),
    // log panel
    LogLevel(&'static str),
    LogCopy,
    LogCopied,
    LogClear,
    LogTall,
    // file browser
    Picker(Pk),
    Noop,
}

/// Blocking backend work on its own thread; the result comes back as a `Msg`
/// on the UI loop's channel (the TUI's `Task::perform(blocking(..))`).
#[derive(Clone)]
pub struct Jobs(pub mpsc::Sender<Msg>);

impl Jobs {
    pub fn run<T, F, M>(&self, f: F, map: M)
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
        M: FnOnce(T) -> Msg + Send + 'static,
    {
        let tx = self.0.clone();
        std::thread::spawn(move || {
            let _ = tx.send(map(f()));
        });
    }
}

pub struct TuiApp {
    jobs: Jobs,
    /// Bumped on every Start over, so results from an abandoned session are dropped.
    gen: u64,
    pub quit: bool,

    pub role: Option<Role>,
    pub step: Step,
    pub link: LinkStatus,
    pub peer: Option<Peer>,
    pub self_peer: Option<Peer>,
    pub code: String,
    pub mode: TransferMode,
    pub sources: Vec<SourceGroup>,
    pub picks: Vec<SourceGroup>,
    pub progress: TransferProgress,
    pub received_dir: Option<String>,

    // connect
    pub adapters: Vec<AdapterInfo>,
    pub fw: Fw,
    last_link_poll: Option<Instant>,
    last_adapter_poll: Option<Instant>,
    // pair
    pub pair_mode: PairMode,
    pub pair_status: PairStatus,
    pub pair_err: String,
    pub code_edit: LineEdit,
    pub addr_edit: LineEdit,
    discover_run: u32,
    // code
    pub ips: Vec<String>,
    pub drop: Option<DropSettings>,
    pub drop_free: Option<u64>,
    pub receive_err: Option<String>,
    // select
    pub picking: bool,
    pub testing: bool,
    pub speed: String,
    // transfer
    pub stopping: bool,
    transfer_started: Option<Instant>,
    pub transfer_took: Option<Duration>,
    last_state: String,
    // done
    pub report: Option<ImportReport>,
    pub importing: bool,
    pub import_err: Option<String>,
    pub pending_browser: Option<String>,
    pub use_folder: bool,
    pub import_folder: Option<String>,
    pub folder_free: Option<u64>,
    // logs
    pub logs_open: bool,
    pub log_tall: bool,
    pub log_floor: &'static str,
    pub filter_edit: LineEdit,
    pub log_rows: Vec<LogEntry>,
    pub log_path: String,
    pub log_copied: bool,
    /// lines scrolled up from the newest (0 = follow the tail)
    pub log_scroll: usize,
    errors_seen: u64,

    // terminal UI
    pub picker: Option<Picker>,
    pub confirm_quit: bool,
    /// focused hit key ("" = none yet)
    pub focus: String,
    /// keyboard moved focus: scroll it into view on the next draw
    pub reveal: bool,
    pub body_scroll: u16,
    /// everything clickable / focusable from the last draw, in Tab order
    pub hits: Vec<Hit>,
    pub body_area: Rect,
    pub log_area: Rect,
    /// "..." animation frame
    pub tick: u64,
}

impl TuiApp {
    pub fn new(jobs: Jobs) -> Self {
        TuiApp {
            jobs,
            gen: 0,
            quit: false,
            role: None,
            step: Step::Connect,
            link: LinkStatus::default(),
            peer: None,
            self_peer: None,
            code: String::new(),
            mode: TransferMode::User,
            sources: Vec::new(),
            picks: Vec::new(),
            progress: idle_progress(),
            received_dir: None,
            adapters: Vec::new(),
            fw: Fw::Idle,
            last_link_poll: None,
            last_adapter_poll: None,
            pair_mode: PairMode::Auto,
            pair_status: PairStatus::Finding,
            pair_err: String::new(),
            code_edit: LineEdit::default(),
            addr_edit: LineEdit::default(),
            discover_run: 0,
            ips: Vec::new(),
            drop: None,
            drop_free: None,
            receive_err: None,
            picking: false,
            testing: false,
            speed: String::new(),
            stopping: false,
            transfer_started: None,
            transfer_took: None,
            last_state: "idle".into(),
            report: None,
            importing: false,
            import_err: None,
            pending_browser: None,
            use_folder: false,
            import_folder: None,
            folder_free: None,
            logs_open: false,
            log_tall: false,
            log_floor: logbus::level(),
            filter_edit: LineEdit::default(),
            log_rows: logbus::entries(),
            log_path: logbus::file_path(),
            log_copied: false,
            log_scroll: 0,
            errors_seen: 0,
            picker: None,
            confirm_quit: false,
            focus: String::new(),
            reveal: false,
            body_scroll: 0,
            hits: Vec::new(),
            body_area: Rect::default(),
            log_area: Rect::default(),
            tick: 0,
        }
    }

    pub fn unseen_errors(&self) -> usize {
        self.log_rows
            .iter()
            .filter(|e| e.level == "error" && e.seq >= self.errors_seen)
            .count()
    }

    /// Who "Pair" would connect to: the found peer, or the typed address.
    pub fn pair_target(&self) -> Option<Peer> {
        match self.pair_mode {
            PairMode::Manual => parse_addr(&self.addr_edit.text),
            PairMode::Auto => self.peer.clone(),
        }
    }

    pub fn pair_ready(&self) -> bool {
        self.code_edit.text.len() == 6 && self.pair_target().is_some() && self.pair_status != PairStatus::Pairing
    }

    /// Quitting now would cut off something in flight (a pairing held open, a
    /// listener a sender may be dialling, a transfer, an import).
    pub fn busy(&self) -> bool {
        let running = matches!(self.progress.state.as_str(), "running" | "reconnecting" | "idle");
        match (self.role, self.step) {
            (None, _) | (Some(_), Step::Connect) => false,
            (Some(_), Step::Transfer) => running,
            (Some(_), Step::Done) => self.importing || self.pending_browser.is_some(),
            _ => true,
        }
    }

    /// Timers the window gets from subscriptions: the link / adapter polls on
    /// the connect step. Returns true if anything was started.
    pub fn timers(&mut self, now: Instant) -> bool {
        let mut any = false;
        if self.role.is_some() && self.step == Step::Connect {
            let due = |last: Option<Instant>, every: u64| last.is_none_or(|t| now - t >= Duration::from_millis(every));
            if due(self.last_link_poll, 1500) {
                self.update(Msg::PollLink);
                any = true;
            }
            if due(self.last_adapter_poll, 2000) {
                self.update(Msg::PollAdapters);
                any = true;
            }
        }
        any
    }

    pub fn update(&mut self, msg: Msg) {
        // every user action at Trace level (not backend pushes or polling)
        if !matches!(
            msg,
            Msg::Backend(_)
                | Msg::PollLink
                | Msg::Link(_)
                | Msg::PollAdapters
                | Msg::Adapters(_)
                | Msg::Noop
        ) {
            let mut line = format!("{msg:?}");
            line.truncate(300);
            log::trace!("tui ← {line}");
        }
        let gen = self.gen;
        match msg {
            /* ---------- shell ---------- */
            Msg::SetRole(r) => {
                self.role = Some(r);
                self.go(Step::Connect);
            }
            Msg::Reset => {
                self.reset();
                self.jobs.run(backend::cancel, |_| Msg::Noop);
            }
            Msg::GoTo(s) => self.go(s),
            Msg::ToggleLogs => self.toggle_logs(),
            Msg::Quit => {
                if self.busy() {
                    self.confirm_quit = true;
                    self.focus = "q:no".into();
                } else {
                    self.quit = true;
                }
            }
            Msg::QuitAnswer(yes) => {
                self.confirm_quit = false;
                self.focus.clear();
                if yes {
                    self.quit = true;
                }
            }
            Msg::Backend(ev) => self.on_backend(ev),

            /* ---------- connect ---------- */
            Msg::PollLink => {
                self.last_link_poll = Some(Instant::now());
                self.jobs.run(backend::link_status, Msg::Link);
            }
            Msg::Link(l) => self.link = l,
            Msg::PollAdapters => {
                self.last_adapter_poll = Some(Instant::now());
                self.jobs.run(backend::list_adapters, Msg::Adapters);
            }
            Msg::Adapters(a) => self.adapters = a,
            Msg::AllowFirewall => {
                self.fw = Fw::Working;
                self.jobs.run(backend::allow_firewall, Msg::Firewall);
            }
            Msg::Firewall(r) => {
                self.fw = match r {
                    Ok(()) => Fw::Done,
                    Err(e) => {
                        ui_log("error", "tui::connect", &format!("firewall rule not added: {e}"));
                        Fw::Error
                    }
                }
            }

            /* ---------- pair ---------- */
            Msg::SetPairMode(m) => {
                self.pair_mode = m;
                self.pair_err.clear();
                if m == PairMode::Auto {
                    self.discover();
                } else {
                    self.pair_status = PairStatus::Found;
                    self.set_focus(IN_ADDR);
                }
            }
            Msg::Discovered(run, found) => {
                if run != self.discover_run || self.pair_mode != PairMode::Auto || self.step != Step::Pair {
                    return;
                }
                match found {
                    Some(peer) => {
                        self.peer = Some(peer);
                        self.pair_status = PairStatus::Found;
                        self.set_focus(IN_CODE);
                    }
                    None => {
                        self.pair_status = PairStatus::Error;
                        self.pair_err =
                            "No PC found on the cable. Switch to “Enter IP address” if you’re on Wi-Fi or Ethernet."
                                .into();
                    }
                }
            }
            Msg::AddrSubmit => self.set_focus(IN_CODE),
            Msg::SubmitPair => self.submit_pair(),
            Msg::Paired(g, r) if g == gen => match r {
                Ok(sources) => {
                    self.sources = sources;
                    self.go(Step::Select);
                }
                Err(e) => {
                    let peer = self.pair_target();
                    ui_log(
                        "error",
                        "tui::pair",
                        &format!(
                            "pairing with {} failed: {e}",
                            peer.map(|p| format!("{}:{}", p.ip, p.port)).unwrap_or_default()
                        ),
                    );
                    self.pair_status = PairStatus::Error;
                    self.pair_err = pair_error_text(&e).into();
                }
            },

            /* ---------- code (receiver) ---------- */
            Msg::ReceiverStarted(g, r) if g == gen => match r {
                Ok(info) => {
                    self.code = info.code;
                    self.self_peer = Some(info.peer);
                    self.jobs.run(backend::receive, move |r| Msg::Received(g, r));
                }
                Err(e) => {
                    ui_log("error", "tui::receive", &format!("could not start the receiver: {e}"));
                    self.receive_err = Some(e);
                }
            },
            Msg::LocalIps(v) => self.ips = v,
            Msg::Drop(d) => {
                let root = d.receive_root.clone();
                self.drop = Some(d);
                self.jobs.run(move || backend::free_space(&root), Msg::DropFree);
            }
            Msg::DropFree(f) => self.drop_free = f,
            Msg::ChangeDrop => {
                let start = self.drop.as_ref().map(|d| d.receive_root.clone());
                self.open_picker(PickKind::Folder, PickFor::Drop, start.as_deref());
            }
            Msg::ResetDrop => self.jobs.run(|| backend::set_receive_root(None), Msg::DropSet),
            Msg::DropSet(r) => match r {
                Ok(d) => self.update(Msg::Drop(d)),
                Err(e) => ui_log("error", "tui::code", &format!("could not change the drop folder: {e}")),
            },
            Msg::Received(g, r) if g == gen => match r {
                Ok(dir) => self.received_dir = Some(dir),
                Err(e) => ui_log("error", "tui::receive", &format!("receive failed: {e}")),
            },

            /* ---------- select (sender) ---------- */
            Msg::SetMode(m) => self.mode = m,
            Msg::Toggle(id) => {
                if let Some(g) = self.sources.iter_mut().find(|g| g.id == id) {
                    g.selected = !g.selected;
                }
            }
            Msg::SelectAll(on) => self.sources.iter_mut().for_each(|g| g.selected = on),
            Msg::Pick(into) => self.open_picker(PickKind::Many { dirs_only: false }, PickFor::Into(into), None),
            Msg::Picked(into, found) => {
                self.picking = false;
                if let Some(found) = found {
                    match into {
                        PickInto::Picks => merge_picks(&mut self.picks, found),
                        PickInto::Sources => {
                            for f in found {
                                if !self.sources.iter().any(|g| g.id == f.id) {
                                    self.sources.push(f);
                                }
                            }
                        }
                    }
                }
            }
            Msg::RemovePick(id) => self.picks.retain(|g| g.id != id),
            Msg::ClearPicks => self.picks.clear(),
            Msg::StartTransfer => self.go(Step::Transfer),
            Msg::RunBench => {
                self.testing = true;
                self.speed.clear();
                self.jobs.run(backend::link_benchmark, move |r| Msg::Bench(gen, r));
            }
            Msg::Bench(g, r) if g == gen => {
                self.testing = false;
                self.speed = match r {
                    Ok(bps) => format!(
                        "≈ {} ({:.1} Gbit/s) — cable, network and encryption, no disk",
                        format::rate(bps),
                        bps * 8.0 / 1e9
                    ),
                    Err(e) => {
                        ui_log("error", "tui::select", &format!("speed test failed: {e}"));
                        e
                    }
                };
            }

            /* ---------- transfer ---------- */
            Msg::Stop => {
                self.stopping = true;
                self.jobs.run(backend::cancel, |_| Msg::Noop);
            }
            Msg::Sent(g, r) if g == gen => {
                if let Err(e) = r {
                    ui_log("error", "tui::transfer", &format!("send failed: {e}"));
                }
            }

            /* ---------- done (receiver import) ---------- */
            Msg::UseImportFolder(on) => self.use_folder = on,
            Msg::ChooseImportFolder => {
                let start = self.import_folder.clone();
                self.open_picker(PickKind::Folder, PickFor::ImportFolder, start.as_deref());
            }
            Msg::ImportFolder(Some(dir)) => {
                self.use_folder = true;
                self.import_folder = Some(dir.clone());
                self.jobs.run(move || backend::free_space(&dir), Msg::ImportFree);
            }
            Msg::ImportFolder(None) => {}
            Msg::ImportFree(f) => self.folder_free = f,
            Msg::RunImport => {
                let Some(dir) = self.received_dir.clone() else { return };
                let target = if self.use_folder { self.import_folder.clone() } else { None };
                if self.use_folder && target.is_none() {
                    return;
                }
                self.importing = true;
                self.import_err = None;
                self.jobs.run(move || backend::import_received(dir, target), Msg::Imported);
            }
            Msg::Imported(r) => {
                self.importing = false;
                match r {
                    Ok(r) => self.report = Some(r),
                    Err(e) => self.import_err = Some(e),
                }
            }
            Msg::Browser(label, action) => {
                let Some(dir) = self.received_dir.clone() else { return };
                if self.pending_browser.is_some() {
                    return;
                }
                self.pending_browser = Some(label.clone());
                self.import_err = None;
                let l = label.clone();
                self.jobs.run(
                    move || match action {
                        BrowserAction::Overwrite => backend::import_browser_overwrite(dir, l),
                        BrowserAction::Retry => backend::import_browser_retry(dir, l),
                    },
                    move |r| Msg::BrowserDone(label, r),
                );
            }
            Msg::BrowserDone(label, r) => {
                self.pending_browser = None;
                match r {
                    Ok(updated) => {
                        if let Some(rep) = &mut self.report {
                            for en in rep.entries.iter_mut() {
                                if en.browser_label.as_deref() == Some(label.as_str()) {
                                    *en = updated.clone();
                                }
                            }
                        }
                    }
                    Err(e) => self.import_err = Some(e),
                }
            }

            /* ---------- log panel ---------- */
            Msg::LogLevel(l) => {
                self.log_floor = l;
                logbus::set_level(l);
            }
            Msg::LogCopy => {
                let text = crate::ui::logview::filter(&self.log_rows, self.log_floor, &self.filter_edit.text)
                    .iter()
                    .map(|e| log_line(e))
                    .collect::<Vec<_>>()
                    .join("\n");
                super::osc52_copy(&text);
                self.log_copied = true;
                self.jobs.run(|| std::thread::sleep(Duration::from_millis(1600)), |_| Msg::LogCopied);
            }
            Msg::LogCopied => self.log_copied = false,
            Msg::LogClear => {
                logbus::clear();
                self.log_rows = logbus::entries();
                self.log_scroll = 0;
            }
            Msg::LogTall => self.log_tall = !self.log_tall,

            /* ---------- file browser ---------- */
            Msg::Picker(pk) => self.on_picker(pk),

            // results from a session the user already abandoned
            Msg::Paired(..)
            | Msg::ReceiverStarted(..)
            | Msg::Received(..)
            | Msg::Bench(..)
            | Msg::Sent(..)
            | Msg::Noop => {}
        }
    }

    fn set_focus(&mut self, key: &str) {
        self.focus = key.into();
        self.reveal = true;
    }

    fn toggle_logs(&mut self) {
        self.logs_open = !self.logs_open;
        if self.logs_open {
            self.log_rows = logbus::entries();
            self.log_scroll = 0;
        } else if self.focus.starts_with("log:") || self.focus == IN_FILTER {
            self.focus.clear();
        }
        self.errors_seen = self.log_rows.last().map_or(0, |e| e.seq + 1);
    }

    /// Enter a step and start whatever backend work it owns.
    fn go(&mut self, step: Step) {
        if self.step != step {
            self.body_scroll = 0;
            self.focus.clear();
        }
        if step == Step::Transfer && self.step != Step::Transfer {
            self.transfer_started = Some(Instant::now());
        }
        self.step = step;
        self.start(step);
    }

    fn start(&mut self, step: Step) {
        let gen = self.gen;
        match (self.role, step) {
            (Some(_), Step::Connect) => {
                self.update(Msg::PollLink);
                self.update(Msg::PollAdapters);
            }
            (Some(Role::Send), Step::Pair) => {
                self.pair_err.clear();
                if self.pair_mode == PairMode::Auto {
                    self.discover();
                } else {
                    self.set_focus(IN_ADDR);
                }
            }
            (Some(Role::Send), Step::Transfer) => {
                self.stopping = false;
                let ids: Vec<String> = match self.mode {
                    TransferMode::Custom => self.picks.iter().map(|g| g.id.clone()).collect(),
                    TransferMode::User => self.sources.iter().filter(|g| g.selected).map(|g| g.id.clone()).collect(),
                };
                if self.peer.is_none() {
                    ui_log("error", "tui::transfer", "reached the transfer step with no paired peer");
                    return;
                }
                let kind = if self.mode == TransferMode::Custom { "custom" } else { "user" };
                ui_log("info", "tui::transfer", &format!("starting {kind} send of {} group(s)", ids.len()));
                self.jobs.run(move || backend::start_send(ids), move |r| Msg::Sent(gen, r));
            }
            (Some(Role::Receive), Step::Code) => {
                self.receive_err = None;
                self.jobs.run(|| backend::start_receiver("This PC"), move |r| Msg::ReceiverStarted(gen, r));
                self.jobs.run(backend::list_local_ips, Msg::LocalIps);
                self.jobs.run(backend::drop_settings, Msg::Drop);
            }
            (Some(Role::Receive), Step::Done) => {
                self.jobs.run(|| backend::drop_settings().import_dir, Msg::ImportFolder);
            }
            _ => {}
        }
    }

    fn discover(&mut self) {
        self.discover_run += 1;
        self.pair_status = PairStatus::Finding;
        let run = self.discover_run;
        self.jobs.run(backend::discover_peer, move |p| Msg::Discovered(run, p));
    }

    fn submit_pair(&mut self) {
        if !self.pair_ready() {
            return;
        }
        let Some(peer) = self.pair_target() else { return };
        let code = self.code_edit.text.clone();
        self.pair_status = PairStatus::Pairing;
        self.pair_err.clear();
        if self.pair_mode == PairMode::Manual {
            self.peer = Some(peer.clone());
        }
        let gen = self.gen;
        self.jobs.run(
            move || {
                backend::pair(peer, code)?;
                Ok(backend::list_sources())
            },
            move |r| Msg::Paired(gen, r),
        );
    }

    fn on_backend(&mut self, ev: Event) {
        match ev {
            Event::Log => {
                self.log_rows = logbus::entries();
                if self.logs_open {
                    self.errors_seen = self.log_rows.last().map_or(0, |e| e.seq + 1);
                }
            }
            Event::Paired(peer) => {
                if self.role == Some(Role::Receive) && self.step == Step::Code {
                    ui_log("info", "tui::receive", &format!("sender {} ({}) connected", peer.name, peer.ip));
                    self.peer = Some(peer);
                    self.go(Step::Transfer);
                }
            }
            Event::Progress(p) => {
                if self.role.is_none() {
                    return;
                }
                if p.state != self.last_state {
                    self.last_state = p.state.clone();
                    let detail = if p.state == "error" {
                        format!(" — {}", p.error.as_deref().unwrap_or("no reason given"))
                    } else {
                        format!(" — {}/{} files", p.files_sent, p.files_total)
                    };
                    let level = if p.state == "error" { "error" } else { "info" };
                    ui_log(level, "tui::transfer", &format!("progress: {}{detail}", p.state));
                }
                let done = p.state == "done";
                if p.state == "error" {
                    self.stopping = false;
                }
                self.progress = p;
                if done {
                    self.transfer_took = self.transfer_started.map(|t| t.elapsed());
                    if let Some(took) = self.transfer_took {
                        ui_log(
                            "info",
                            "tui::transfer",
                            &format!("transfer took {}", format::duration(took.as_secs_f64())),
                        );
                    }
                    self.go(Step::Done);
                }
            }
        }
    }

    /// Back to the role picker. The caller also stops the backend session.
    fn reset(&mut self) {
        let mut fresh = TuiApp::new(self.jobs.clone());
        fresh.gen = self.gen + 1;
        fresh.logs_open = self.logs_open;
        fresh.log_tall = self.log_tall;
        fresh.log_floor = self.log_floor;
        fresh.filter_edit = std::mem::take(&mut self.filter_edit);
        fresh.log_rows = std::mem::take(&mut self.log_rows);
        fresh.errors_seen = self.errors_seen;
        fresh.tick = self.tick;
        *self = fresh;
    }

    /* ---------------- file browser ---------------- */

    fn open_picker(&mut self, kind: PickKind, purpose: PickFor, start: Option<&str>) {
        self.picker = Some(Picker::open(kind, purpose, start));
        self.focus = PK_LIST.into();
    }

    fn on_picker(&mut self, pk: Pk) {
        let Some(p) = self.picker.as_mut() else { return };
        match pk {
            Pk::Click(i) => {
                p.click(i);
                self.focus = PK_LIST.into();
            }
            Pk::Mark(i) => {
                p.click_mark(i);
                self.focus = PK_LIST.into();
            }
            Pk::Parent => p.parent(),
            Pk::Home => p.cd(super::picker::home()),
            Pk::Hidden => p.toggle_hidden(),
            Pk::PathSubmit => p.submit_path(),
            Pk::Cancel => {
                self.picker = None;
                self.focus.clear();
            }
            Pk::Confirm => {
                let Some((paths, _)) = p.result() else { return };
                let purpose = p.purpose;
                self.picker = None;
                self.focus.clear();
                log::info!("picked {} path(s): {}", paths.len(), paths.join(", "));
                match purpose {
                    PickFor::Drop => {
                        let dir = paths.into_iter().next();
                        self.jobs.run(move || backend::set_receive_root(dir), Msg::DropSet);
                    }
                    PickFor::ImportFolder => self.update(Msg::ImportFolder(paths.into_iter().next())),
                    PickFor::Into(into) => {
                        self.picking = true;
                        self.jobs.run(move || Some(backend::measure_paths(paths)), move |f| Msg::Picked(into, f));
                    }
                }
            }
        }
    }

    /* ---------------- terminal input ---------------- */

    fn focused_hit(&self) -> Option<&Hit> {
        self.hits.iter().find(|h| h.key == self.focus)
    }

    /// Tab order: distinct keys in the order they were drawn.
    fn move_focus(&mut self, forward: bool) {
        let mut keys: Vec<&str> = Vec::new();
        for h in &self.hits {
            if keys.last() != Some(&h.key.as_str()) && !keys.contains(&h.key.as_str()) {
                keys.push(&h.key);
            }
        }
        if keys.is_empty() {
            return;
        }
        let next = match keys.iter().position(|k| *k == self.focus) {
            Some(i) if forward => keys[(i + 1) % keys.len()],
            Some(i) => keys[(i + keys.len() - 1) % keys.len()],
            None if forward => keys[0],
            None => keys[keys.len() - 1],
        }
        .to_string();
        self.focus = next;
        self.reveal = true;
    }

    fn activate_focused(&mut self) {
        if let Some(h) = self.focused_hit() {
            let on = h.on.clone();
            self.update(on);
        }
    }

    fn edit_target(&mut self) -> Option<&mut LineEdit> {
        match self.focus.as_str() {
            IN_CODE if self.pair_status != PairStatus::Pairing => Some(&mut self.code_edit),
            IN_ADDR => Some(&mut self.addr_edit),
            IN_FILTER => Some(&mut self.filter_edit),
            IN_PATH => self.picker.as_mut().map(|p| &mut p.path),
            _ => None,
        }
    }

    /// After typing into a field: keep the code to 6 digits.
    fn after_edit(&mut self) {
        if self.focus == IN_CODE {
            let digits: String = self.code_edit.text.chars().filter(|c| c.is_ascii_digit()).take(6).collect();
            if digits != self.code_edit.text {
                self.code_edit.set(digits);
            }
        }
        if self.focus == IN_FILTER {
            self.log_scroll = 0;
        }
    }

    pub fn paste(&mut self, s: &str) {
        if let Some(e) = self.edit_target() {
            e.insert(s);
            self.after_edit();
        }
    }

    pub fn key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('q')) {
            if self.confirm_quit {
                return self.update(Msg::QuitAnswer(true));
            }
            return self.update(Msg::Quit);
        }
        if self.confirm_quit {
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => return self.update(Msg::QuitAnswer(true)),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => return self.update(Msg::QuitAnswer(false)),
                _ => {}
            }
        } else if ctrl && k.code == KeyCode::Char('l') && self.picker.is_none() {
            return self.update(Msg::ToggleLogs);
        }

        // typing into a field
        let in_field = self.focused_hit().is_some_and(|h| h.kind == Kind::Input) || self.focus == IN_CODE;
        if in_field {
            if self.focus == IN_PATH && k.code == KeyCode::Tab {
                if let Some(p) = self.picker.as_mut() {
                    if p.complete_path() {
                        return;
                    }
                }
            }
            if let Some(e) = self.edit_target() {
                if e.key(&k) {
                    self.after_edit();
                    return;
                }
            }
        }

        // the file browser's list
        if self.focus == PK_LIST {
            if let Some(p) = self.picker.as_mut() {
                let page = p.page.max(1) as isize;
                let handled = match k.code {
                    KeyCode::Up => {
                        p.move_by(-1);
                        true
                    }
                    KeyCode::Down => {
                        p.move_by(1);
                        true
                    }
                    KeyCode::PageUp => {
                        p.move_by(-page);
                        true
                    }
                    KeyCode::PageDown => {
                        p.move_by(page);
                        true
                    }
                    KeyCode::Home => {
                        p.move_by(-(p.entries.len() as isize));
                        true
                    }
                    KeyCode::End => {
                        p.move_by(p.entries.len() as isize);
                        true
                    }
                    KeyCode::Enter | KeyCode::Right => {
                        p.activate();
                        true
                    }
                    KeyCode::Backspace | KeyCode::Left => {
                        p.parent();
                        true
                    }
                    KeyCode::Char(' ') => {
                        p.toggle_mark();
                        true
                    }
                    KeyCode::Char('.') => {
                        p.toggle_hidden();
                        true
                    }
                    KeyCode::Char('~') => {
                        p.cd(super::picker::home());
                        true
                    }
                    KeyCode::Char('/') => {
                        self.focus = IN_PATH.into();
                        true
                    }
                    _ => false,
                };
                if handled {
                    return;
                }
            }
        }

        match k.code {
            KeyCode::Esc => {
                if self.picker.is_some() {
                    self.on_picker(Pk::Cancel);
                } else if self.logs_open {
                    self.toggle_logs();
                }
            }
            KeyCode::Tab | KeyCode::Down => self.move_focus(true),
            KeyCode::BackTab | KeyCode::Up => self.move_focus(false),
            KeyCode::Right if !in_field => self.move_focus(true),
            KeyCode::Left if !in_field => self.move_focus(false),
            KeyCode::Enter => self.activate_focused(),
            KeyCode::Char(' ') if !in_field => self.activate_focused(),
            KeyCode::PageUp | KeyCode::PageDown => {
                let up = k.code == KeyCode::PageUp;
                if self.logs_open && (self.focus.starts_with("log:") || self.focus == IN_FILTER) {
                    self.scroll_log(if up { 10 } else { -10 });
                } else {
                    let page = self.body_area.height.saturating_sub(2).max(1);
                    self.body_scroll = if up {
                        self.body_scroll.saturating_sub(page)
                    } else {
                        self.body_scroll.saturating_add(page)
                    };
                }
            }
            KeyCode::Home if !in_field => self.body_scroll = 0,
            KeyCode::End if !in_field => self.body_scroll = u16::MAX,
            KeyCode::Char('q') if !in_field && self.picker.is_none() => self.update(Msg::Quit),
            KeyCode::Char('s') if self.role.is_none() && !in_field => self.update(Msg::SetRole(Role::Send)),
            KeyCode::Char('r') if self.role.is_none() && !in_field => self.update(Msg::SetRole(Role::Receive)),
            // digits on the pair step go to the code wherever focus is
            KeyCode::Char(c) if c.is_ascii_digit() && self.step == Step::Pair && self.role == Some(Role::Send) && !in_field && self.picker.is_none() => {
                self.focus = IN_CODE.into();
                self.code_edit.insert(c.encode_utf8(&mut [0; 4]));
                self.after_edit();
            }
            _ => {}
        }
    }

    fn scroll_log(&mut self, delta: isize) {
        self.log_scroll = (self.log_scroll as isize + delta).max(0) as usize;
    }

    pub fn mouse(&mut self, m: MouseEvent) {
        let at = Position::new(m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let modal = self.picker.is_some() || self.confirm_quit;
                let hit = self
                    .hits
                    .iter()
                    .find(|h| (!modal || h.region == Region::Modal) && h.screen.is_some_and(|r| r.contains(at)))
                    .cloned();
                if let Some(h) = hit {
                    self.focus = h.key.clone();
                    if h.kind == Kind::Input {
                        if let Some(e) = self.edit_target() {
                            e.cursor = e.len();
                        }
                    } else {
                        self.update(h.on);
                    }
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = m.kind == MouseEventKind::ScrollUp;
                if let Some(p) = self.picker.as_mut() {
                    p.scroll_by(if up { -3 } else { 3 });
                } else if self.logs_open && self.log_area.contains(at) {
                    self.scroll_log(if up { 3 } else { -3 });
                } else {
                    self.body_scroll = if up {
                        self.body_scroll.saturating_sub(3)
                    } else {
                        self.body_scroll.saturating_add(3)
                    };
                }
            }
            _ => {}
        }
    }
}

pub fn ui_log(level: &str, target: &str, msg: &str) {
    logbus::ui(level, target, msg);
}

/// One log row as text, for Copy.
pub fn log_line(e: &LogEntry) -> String {
    format!("{} {:<5} [{}] {}", format::clock(e.ts), e.level.to_uppercase(), e.target, e.msg)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An app whose backend jobs report into a channel the test can read.
    pub fn app() -> (TuiApp, mpsc::Receiver<Msg>) {
        let (tx, rx) = mpsc::channel();
        (TuiApp::new(Jobs(tx)), rx)
    }

    fn group(id: &str, selected: bool) -> SourceGroup {
        SourceGroup {
            id: id.into(),
            label: id.into(),
            hint: format!("/home/u/{id}"),
            kind: "folder".into(),
            path: Some(format!("/home/u/{id}")),
            bytes: 2_000_000,
            items: 12,
            caveat: None,
            selected,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn hit(key: &str, kind: Kind, on: Msg, y: u16) -> Hit {
        Hit {
            key: key.into(),
            on,
            kind,
            region: Region::Body,
            virt: Rect::new(0, y, 10, 1),
            screen: Some(Rect::new(0, y, 10, 1)),
        }
    }

    #[test]
    fn focus_walks_hits_and_enter_fires() {
        let (mut a, _rx) = app();
        a.sources = vec![group("documents", true), group("music", false)];
        a.hits = vec![
            hit("src:documents", Kind::Toggle, Msg::Toggle("documents".into()), 0),
            hit("src:music", Kind::Toggle, Msg::Toggle("music".into()), 1),
            hit("all", Kind::Button, Msg::SelectAll(true), 2),
        ];
        a.key(key(KeyCode::Tab));
        assert_eq!(a.focus, "src:documents");
        a.key(key(KeyCode::Down));
        a.key(key(KeyCode::Char(' ')));
        assert!(a.sources[1].selected);
        a.key(key(KeyCode::BackTab));
        a.key(key(KeyCode::BackTab));
        assert_eq!(a.focus, "all"); // wraps around
        a.key(key(KeyCode::Enter));
        assert!(a.sources.iter().all(|g| g.selected));
    }

    #[test]
    fn clicks_resolve_through_screen_rects() {
        let (mut a, _rx) = app();
        a.sources = vec![group("documents", false)];
        a.hits = vec![hit("src:documents", Kind::Toggle, Msg::Toggle("documents".into()), 4)];
        let click = |row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row,
            modifiers: KeyModifiers::NONE,
        };
        a.mouse(click(5));
        assert!(!a.sources[0].selected);
        a.mouse(click(4));
        assert!(a.sources[0].selected);
        assert_eq!(a.focus, "src:documents");
    }

    #[test]
    fn code_field_keeps_six_digits() {
        let (mut a, _rx) = app();
        a.role = Some(Role::Send);
        a.step = Step::Pair;
        // digits typed anywhere on the pair step land in the code
        a.key(key(KeyCode::Char('4')));
        assert_eq!(a.focus, IN_CODE);
        a.hits = vec![hit(IN_CODE, Kind::Input, Msg::SubmitPair, 0)];
        for c in "1x8 6201".chars() {
            a.key(key(KeyCode::Char(c)));
        }
        assert_eq!(a.code_edit.text, "418620");
        a.paste("99");
        assert_eq!(a.code_edit.text, "418620");
        a.key(key(KeyCode::Backspace));
        assert_eq!(a.code_edit.text, "41862");
        // 'q' while typing is text, not quit
        a.key(key(KeyCode::Char('q')));
        assert!(!a.quit && !a.confirm_quit);
    }

    #[test]
    fn manual_pair_needs_ip_and_code() {
        let (mut a, _rx) = app();
        a.pair_mode = PairMode::Manual;
        a.pair_status = PairStatus::Found;
        a.addr_edit.set("192.168.1");
        a.code_edit.set("123456");
        assert!(!a.pair_ready());
        a.addr_edit.set("127.0.0.1");
        assert!(a.pair_ready());
        assert_eq!(a.pair_target().unwrap().port, crate::model::TRANSFER_PORT);
    }

    #[test]
    fn quit_asks_first_when_busy() {
        let (mut a, _rx) = app();
        a.update(Msg::Quit);
        assert!(a.quit, "nothing in flight on the role picker");

        let (mut a, _rx) = app();
        a.role = Some(Role::Receive);
        a.step = Step::Code;
        a.key(key(KeyCode::Char('q')));
        assert!(a.confirm_quit && !a.quit);
        a.key(key(KeyCode::Char('n')));
        assert!(!a.confirm_quit && !a.quit);
        a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.quit);
    }

    #[test]
    fn stale_results_are_dropped_after_start_over() {
        let (mut a, _rx) = app();
        a.role = Some(Role::Send);
        a.step = Step::Pair;
        a.update(Msg::Reset);
        assert!(a.role.is_none());
        a.update(Msg::Paired(0, Ok(vec![group("documents", true)])));
        assert!(a.sources.is_empty());
        assert_eq!(a.step, Step::Connect);
    }

    #[test]
    fn picker_results_route_by_purpose() {
        let (mut a, rx) = app();
        let dir = std::env::temp_dir();
        a.picker = Some(Picker::open(PickKind::Folder, PickFor::ImportFolder, Some(&dir.display().to_string())));
        a.update(Msg::Picker(Pk::Confirm));
        assert!(a.picker.is_none());
        assert!(a.use_folder);
        assert_eq!(a.import_folder.as_deref(), Some(dir.display().to_string().as_str()));
        // free space comes back on the jobs channel
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)), Ok(Msg::ImportFree(_))));

        // Esc cancels without picking
        a.picker = Some(Picker::open(PickKind::Folder, PickFor::Drop, None));
        a.focus = PK_LIST.into();
        a.key(key(KeyCode::Esc));
        assert!(a.picker.is_none());
    }

    #[test]
    fn user_transfer_adds_files_too() {
        let (mut a, rx) = app();
        let file = std::env::temp_dir().join(format!("winci-user-pick-{}.txt", std::process::id()));
        std::fs::write(&file, b"hello").unwrap();
        a.update(Msg::Pick(PickInto::Sources));
        let p = a.picker.as_mut().unwrap();
        assert_eq!(p.kind, PickKind::Many { dirs_only: false });
        p.path.set(file.display().to_string());
        p.submit_path();
        assert_eq!(p.marked, vec![file.clone()]);
        a.update(Msg::Picker(Pk::Confirm));
        let found = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        a.update(found);
        let g = a.sources.iter().find(|g| g.path.as_deref() == Some(file.display().to_string().as_str())).unwrap();
        assert_eq!((g.kind.as_str(), g.bytes, g.selected), ("file", 5, true));
        let _ = std::fs::remove_file(&file);
    }
}
