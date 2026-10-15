//! Native UI (iced, CPU-rendered). One state struct, one message enum, one
//! update — the old Leptos store + per-step signals + SessionController
//! folded together.

mod format;
mod logview;
mod shell;
mod steps;
mod style;
mod widgets;

use crate::backend::{self, blocking, DropSettings, Event, ReceiverInfo};
use crate::import::{ImportEntry, ImportReport};
use crate::logbus::{self, LogEntry};
use crate::model::{AdapterInfo, LinkStatus, Peer, SourceGroup, TransferProgress, TRANSFER_PORT};
use iced::futures::StreamExt;
use iced::widget::operation;
use iced::{keyboard, time, window, Element, Subscription, Task, Theme};
use std::time::Duration;

pub fn run() -> iced::Result {
    let icon = image::load_from_memory(include_bytes!("../../icons/128x128.png"))
        .ok()
        .map(|i| i.into_rgba8())
        .and_then(|i| {
            let (w, h) = i.dimensions();
            window::icon::from_rgba(i.into_raw(), w, h).ok()
        });
    iced::application(App::boot, App::update, App::view)
        .title(|_: &App| format!("WINCI — Data Crossing  ·  v{}", env!("CARGO_PKG_VERSION")))
        .theme(App::theme)
        .subscription(App::subscription)
        .default_font(style::BODY)
        .window(window::Settings {
            size: iced::Size::new(1060.0, 780.0),
            min_size: Some(iced::Size::new(760.0, 620.0)),
            icon,
            ..Default::default()
        })
        .centered()
        .run()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Send,
    Receive,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Connect,
    Pair,
    Select,
    Code,
    Transfer,
    Done,
}

/// Sender's payload: preset user data, or explorer-picked files/folders.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransferMode {
    User,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fw {
    Idle,
    Working,
    Done,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PairMode {
    Auto,
    Manual,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PairStatus {
    Finding,
    Found,
    Pairing,
    Error,
}

/// Which list an Explorer pick lands in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickInto {
    Picks,
    Sources,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BrowserAction {
    Overwrite,
    Retry,
}

pub const CODE_INPUT: &str = "paircode";
pub const LOG_SCROLL: &str = "logscroll";
pub const MAIN_SCROLL: &str = "mainscroll";

#[derive(Debug, Clone)]
pub enum Message {
    // shell
    SetRole(Role),
    Reset,
    GoTo(Step),
    ToggleLogs,
    Backend(Event),
    SystemTheme(iced::theme::Mode),
    SystemInfo(Box<iced::system::Information>),
    Key(keyboard::Event),
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
    CodeInput(String),
    AddrInput(String),
    AddrSubmit,
    SubmitPair,
    Paired(u64, Result<Vec<SourceGroup>, String>),
    // code (receiver)
    ReceiverStarted(u64, Result<ReceiverInfo, String>),
    LocalIps(Vec<String>),
    Drop(DropSettings),
    DropFree(Option<u64>),
    ChangeDrop,
    DropPicked(Option<String>),
    ResetDrop,
    DropSet(Result<DropSettings, String>),
    Received(u64, Result<String, String>),
    // select (sender)
    SetMode(TransferMode),
    Toggle(String),
    SelectAll(bool),
    Pick(bool, PickInto),
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
    // log drawer
    LogLevel(&'static str),
    LogFilter(String),
    LogCopy,
    LogCopied,
    LogClear,
    LogOpenFile,
    LogTall,
    Noop,
}

pub struct App {
    dark: bool,
    /// Bumped on every Start over, so results from an abandoned session are dropped.
    gen: u64,

    pub role: Option<Role>,
    pub step: Step,
    pub link: LinkStatus,
    pub peer: Option<Peer>,
    /// receiver's own address, shown for manual pairing
    pub self_peer: Option<Peer>,
    /// shown (receive) or entered (send), display form
    pub code: String,
    pub mode: TransferMode,
    pub sources: Vec<SourceGroup>,
    /// Custom Transfer list, all sent (no checkboxes)
    pub picks: Vec<SourceGroup>,
    pub progress: TransferProgress,
    /// where the crossing landed, for "Import into place"
    pub received_dir: Option<String>,

    // connect
    pub adapters: Vec<AdapterInfo>,
    pub fw: Fw,
    // pair
    pub pair_mode: PairMode,
    pub pair_status: PairStatus,
    pub pair_err: String,
    pub code_input: String,
    pub addr_input: String,
    discover_run: u32,
    // code
    pub ips: Vec<String>,
    pub drop: Option<DropSettings>,
    pub drop_free: Option<u64>,
    // select
    pub picking: bool,
    pub testing: bool,
    pub speed: String,
    // transfer
    pub stopping: bool,
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
    pub log_filter: String,
    pub log_rows: Vec<LogEntry>,
    pub log_path: String,
    pub log_copied: bool,
    /// errors already seen by opening the drawer (badge = the rest)
    errors_seen: u64,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let app = App {
            dark: false,
            gen: 0,
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
            pair_mode: PairMode::Auto,
            pair_status: PairStatus::Finding,
            pair_err: String::new(),
            code_input: String::new(),
            addr_input: String::new(),
            discover_run: 0,
            ips: Vec::new(),
            drop: None,
            drop_free: None,
            picking: false,
            testing: false,
            speed: String::new(),
            stopping: false,
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
            log_filter: String::new(),
            log_rows: logbus::entries(),
            log_path: logbus::file_path(),
            log_copied: false,
            errors_seen: 0,
        };
        let boot = Task::batch([
            iced::system::theme().map(Message::SystemTheme),
            iced::system::information().map(|i| Message::SystemInfo(Box::new(i))),
        ]);
        (app, boot)
    }

    pub fn pal(&self) -> style::Pal {
        if self.dark {
            style::DARK
        } else {
            style::LIGHT
        }
    }

    fn theme(&self) -> Theme {
        style::theme(&self.pal(), self.dark)
    }

    pub fn flowing(&self) -> bool {
        self.progress.state == "running"
    }

    /// Errors logged since the drawer was last open.
    pub fn unseen_errors(&self) -> usize {
        self.log_rows
            .iter()
            .filter(|e| e.level == "error" && e.seq >= self.errors_seen)
            .count()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            Subscription::run(backend_events),
            iced::system::theme_changes().map(Message::SystemTheme),
            keyboard::listen().map(Message::Key),
        ];
        // APIPA (169.254) can take several seconds to self-assign after the
        // cable links, so poll — a one-shot check misses the cable and
        // latches onto Wi-Fi.
        if self.role.is_some() && self.step == Step::Connect {
            subs.push(time::every(Duration::from_millis(1500)).map(|_| Message::PollLink));
            subs.push(time::every(Duration::from_millis(2000)).map(|_| Message::PollAdapters));
        }
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Message> {
        shell::view(self)
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        // every user action at Trace level (not backend pushes or polling —
        // those are logged where they're handled)
        if !matches!(
            msg,
            Message::Backend(_)
                | Message::SystemInfo(_)
                | Message::Key(_)
                | Message::PollLink
                | Message::Link(_)
                | Message::PollAdapters
                | Message::Adapters(_)
        ) {
            let mut line = format!("{msg:?}");
            line.truncate(300);
            log::trace!("ui ← {line}");
        }
        let gen = self.gen;
        match msg {
            /* ---------- shell ---------- */
            Message::SetRole(r) => {
                self.role = Some(r);
                return self.go(Step::Connect);
            }
            Message::Reset => {
                self.reset();
                return Task::perform(blocking(backend::cancel), |_| Message::Noop);
            }
            Message::GoTo(s) => return self.go(s),
            Message::ToggleLogs => self.toggle_logs(),
            Message::SystemTheme(mode) => self.dark = mode == iced::theme::Mode::Dark,
            Message::SystemInfo(i) => {
                // which renderer this PC ended up on — the first thing to
                // check when someone reports a slow or garbled window
                log::info!(
                    "renderer: {} ({}) · {} {} · {} × {} cores",
                    i.graphics_backend,
                    i.graphics_adapter,
                    i.system_name.as_deref().unwrap_or("?"),
                    i.system_version.as_deref().unwrap_or(""),
                    i.cpu_brand.trim(),
                    i.cpu_cores.map_or("?".into(), |c| c.to_string()),
                );
            }
            Message::Key(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                use keyboard::key::{Key, Named};
                match key.as_ref() {
                    Key::Character("l") if modifiers.command() => self.toggle_logs(),
                    Key::Named(Named::Escape) if self.logs_open => self.logs_open = false,
                    _ => {}
                }
            }
            Message::Key(_) => {}
            Message::Backend(ev) => return self.on_backend(ev),

            /* ---------- connect ---------- */
            Message::PollLink => return Task::perform(blocking(backend::link_status), Message::Link),
            Message::Link(l) => self.link = l,
            Message::PollAdapters => {
                return Task::perform(blocking(backend::list_adapters), Message::Adapters)
            }
            Message::Adapters(a) => self.adapters = a,
            Message::AllowFirewall => {
                self.fw = Fw::Working;
                return Task::perform(blocking(backend::allow_firewall), Message::Firewall);
            }
            Message::Firewall(r) => {
                self.fw = match r {
                    Ok(()) => Fw::Done,
                    Err(e) => {
                        ui_log("error", "ui::connect", &format!("firewall rule not added: {e}"));
                        Fw::Error
                    }
                }
            }

            /* ---------- pair ---------- */
            Message::SetPairMode(m) => {
                self.pair_mode = m;
                self.pair_err.clear();
                if m == PairMode::Auto {
                    return self.discover();
                }
                self.pair_status = PairStatus::Found;
            }
            Message::Discovered(run, found) => {
                if run != self.discover_run || self.pair_mode != PairMode::Auto || self.step != Step::Pair {
                    return Task::none();
                }
                match found {
                    Some(peer) => {
                        self.peer = Some(peer);
                        self.pair_status = PairStatus::Found;
                        return operation::focus(CODE_INPUT);
                    }
                    None => {
                        self.pair_status = PairStatus::Error;
                        self.pair_err =
                            "No PC found on the cable. Switch to “Enter IP” if you’re on Wi-Fi or Ethernet.".into();
                    }
                }
            }
            Message::CodeInput(s) => {
                self.code_input = s.chars().filter(|c| c.is_ascii_digit()).take(6).collect();
            }
            Message::AddrInput(s) => self.addr_input = s,
            Message::AddrSubmit => return operation::focus(CODE_INPUT),
            Message::SubmitPair => return self.submit_pair(),
            Message::Paired(g, r) if g == gen => match r {
                Ok(sources) => {
                    self.sources = sources;
                    return self.go(Step::Select);
                }
                Err(e) => {
                    let peer = self.pair_target();
                    ui_log(
                        "error",
                        "ui::pair",
                        &format!(
                            "pairing with {} failed: {e}",
                            peer.map(|p| format!("{}:{}", p.ip, p.port)).unwrap_or_default()
                        ),
                    );
                    self.pair_status = PairStatus::Error;
                    let lower = e.to_lowercase();
                    self.pair_err = if lower.contains("wrong code") || lower.contains("mismatch") {
                        "That code didn’t match. Re-read it on the new PC."
                    } else {
                        "Couldn’t reach that PC. Check the IP, that WINCI is receiving, and the firewall."
                    }
                    .into();
                }
            },

            /* ---------- code (receiver) ---------- */
            Message::ReceiverStarted(g, r) if g == gen => match r {
                Ok(info) => {
                    self.code = info.code;
                    self.self_peer = Some(info.peer);
                    // the listener exists now — wait for the sender on it
                    return Task::perform(blocking(backend::receive), move |r| Message::Received(g, r));
                }
                Err(e) => ui_log("error", "ui::receive", &format!("could not start the receiver: {e}")),
            },
            Message::LocalIps(v) => self.ips = v,
            Message::Drop(d) => {
                let root = d.receive_root.clone();
                self.drop = Some(d);
                return Task::perform(blocking(move || backend::free_space(&root)), Message::DropFree);
            }
            Message::DropFree(f) => self.drop_free = f,
            Message::ChangeDrop => return Task::perform(blocking(backend::pick_folder), Message::DropPicked),
            Message::DropPicked(Some(dir)) => {
                return Task::perform(blocking(move || backend::set_receive_root(Some(dir))), Message::DropSet)
            }
            Message::DropPicked(None) => {}
            Message::ResetDrop => {
                return Task::perform(blocking(|| backend::set_receive_root(None)), Message::DropSet)
            }
            Message::DropSet(r) => match r {
                Ok(d) => return self.update(Message::Drop(d)),
                Err(e) => ui_log("error", "ui::code", &format!("could not change the drop folder: {e}")),
            },
            Message::Received(g, r) if g == gen => match r {
                Ok(dir) => self.received_dir = Some(dir),
                Err(e) => ui_log("error", "ui::receive", &format!("receive failed: {e}")),
            },

            /* ---------- select (sender) ---------- */
            Message::SetMode(m) => self.mode = m,
            Message::Toggle(id) => {
                if let Some(g) = self.sources.iter_mut().find(|g| g.id == id) {
                    g.selected = !g.selected;
                }
            }
            Message::SelectAll(on) => self.sources.iter_mut().for_each(|g| g.selected = on),
            Message::Pick(directory, into) => {
                self.picking = true;
                return Task::perform(
                    blocking(move || {
                        let paths = backend::pick_paths(directory);
                        (!paths.is_empty()).then(|| backend::measure_paths(paths))
                    }),
                    move |found| Message::Picked(into, found),
                );
            }
            Message::Picked(into, found) => {
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
            Message::RemovePick(id) => self.picks.retain(|g| g.id != id),
            Message::ClearPicks => self.picks.clear(),
            Message::StartTransfer => return self.go(Step::Transfer),
            Message::RunBench => {
                self.testing = true;
                self.speed.clear();
                return Task::perform(blocking(backend::link_benchmark), move |r| Message::Bench(gen, r));
            }
            Message::Bench(g, r) if g == gen => {
                self.testing = false;
                self.speed = match r {
                    Ok(bps) => format!(
                        "≈ {} ({:.1} Gbit/s) — cable, network and encryption, no disk",
                        format::rate(bps),
                        bps * 8.0 / 1e9
                    ),
                    Err(e) => {
                        ui_log("error", "ui::select", &format!("speed test failed: {e}"));
                        e
                    }
                };
            }

            /* ---------- transfer ---------- */
            Message::Stop => {
                self.stopping = true;
                return Task::perform(blocking(backend::cancel), |_| Message::Noop);
            }
            Message::Sent(g, r) if g == gen => {
                if let Err(e) = r {
                    ui_log("error", "ui::transfer", &format!("send failed: {e}"));
                }
            }

            /* ---------- done (receiver import) ---------- */
            Message::UseImportFolder(on) => self.use_folder = on,
            Message::ChooseImportFolder => {
                return Task::perform(blocking(backend::pick_folder), Message::ImportFolder)
            }
            Message::ImportFolder(Some(dir)) => {
                self.use_folder = true;
                self.import_folder = Some(dir.clone());
                return Task::perform(blocking(move || backend::free_space(&dir)), Message::ImportFree);
            }
            Message::ImportFolder(None) => {}
            Message::ImportFree(f) => self.folder_free = f,
            Message::RunImport => {
                let Some(dir) = self.received_dir.clone() else { return Task::none() };
                let target = if self.use_folder { self.import_folder.clone() } else { None };
                if self.use_folder && target.is_none() {
                    return Task::none();
                }
                self.importing = true;
                self.import_err = None;
                return Task::perform(
                    blocking(move || backend::import_received(dir, target)),
                    Message::Imported,
                );
            }
            Message::Imported(r) => {
                self.importing = false;
                match r {
                    Ok(r) => self.report = Some(r),
                    Err(e) => self.import_err = Some(e),
                }
            }
            Message::Browser(label, action) => {
                let Some(dir) = self.received_dir.clone() else { return Task::none() };
                if self.pending_browser.is_some() {
                    return Task::none();
                }
                self.pending_browser = Some(label.clone());
                self.import_err = None;
                let l = label.clone();
                return Task::perform(
                    blocking(move || match action {
                        BrowserAction::Overwrite => backend::import_browser_overwrite(dir, l),
                        BrowserAction::Retry => backend::import_browser_retry(dir, l),
                    }),
                    move |r| Message::BrowserDone(label.clone(), r),
                );
            }
            Message::BrowserDone(label, r) => {
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

            /* ---------- log drawer ---------- */
            Message::LogLevel(l) => {
                self.log_floor = l;
                // debug/trace must also be raised at the source, or those
                // records are never recorded at all
                logbus::set_level(l);
            }
            Message::LogFilter(s) => self.log_filter = s,
            Message::LogCopy => {
                let text = logview::shown(self)
                    .iter()
                    .map(|e| {
                        format!("{} {:<5} [{}] {}", format::clock(e.ts), e.level.to_uppercase(), e.target, e.msg)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.log_copied = true;
                return Task::batch([
                    iced::clipboard::write(text),
                    Task::perform(
                        blocking(|| std::thread::sleep(Duration::from_millis(1600))),
                        |_| Message::LogCopied,
                    ),
                ]);
            }
            Message::LogCopied => self.log_copied = false,
            Message::LogClear => {
                logbus::clear();
                self.log_rows = logbus::entries();
            }
            Message::LogOpenFile => {
                if let Err(e) = logbus::open_dir() {
                    ui_log("error", "ui::log", &format!("could not reveal the log file: {e}"));
                }
            }
            Message::LogTall => self.log_tall = !self.log_tall,

            // results from a session the user already abandoned
            Message::Paired(..)
            | Message::ReceiverStarted(..)
            | Message::Received(..)
            | Message::Bench(..)
            | Message::Sent(..)
            | Message::Noop => {}
        }
        Task::none()
    }

    fn toggle_logs(&mut self) {
        self.logs_open = !self.logs_open;
        if self.logs_open {
            self.log_rows = logbus::entries();
        }
        self.errors_seen = self.log_rows.last().map_or(0, |e| e.seq + 1);
    }

    /// Enter a step and start whatever backend work it owns.
    fn go(&mut self, step: Step) -> Task<Message> {
        // each step starts at its top, not wherever the last one was scrolled to
        let top = if self.step != step {
            operation::snap_to(MAIN_SCROLL, iced::widget::scrollable::RelativeOffset::START)
        } else {
            Task::none()
        };
        self.step = step;
        top.chain(self.start(step))
    }

    /// The backend work a step owns, started when the step is entered.
    fn start(&mut self, step: Step) -> Task<Message> {
        let gen = self.gen;
        match (self.role, step) {
            (Some(_), Step::Connect) => Task::batch([
                self.update(Message::PollLink),
                self.update(Message::PollAdapters),
            ]),
            (Some(Role::Send), Step::Pair) => {
                self.pair_err.clear();
                if self.pair_mode == PairMode::Auto {
                    self.discover()
                } else {
                    Task::none()
                }
            }
            // SENDER: fire the stream once we reach the transfer step
            (Some(Role::Send), Step::Transfer) => {
                self.stopping = false;
                let ids: Vec<String> = match self.mode {
                    TransferMode::Custom => self.picks.iter().map(|g| g.id.clone()).collect(),
                    TransferMode::User => {
                        self.sources.iter().filter(|g| g.selected).map(|g| g.id.clone()).collect()
                    }
                };
                if self.peer.is_none() {
                    ui_log("error", "ui::transfer", "reached the transfer step with no paired peer");
                    return Task::none();
                }
                let kind = if self.mode == TransferMode::Custom { "custom" } else { "user" };
                ui_log("info", "ui::transfer", &format!("starting {kind} send of {} group(s)", ids.len()));
                Task::perform(blocking(move || backend::start_send(ids)), move |r| Message::Sent(gen, r))
            }
            // RECEIVER: open the listener + code as soon as the code step shows
            (Some(Role::Receive), Step::Code) => Task::batch([
                Task::perform(blocking(|| backend::start_receiver("This PC")), move |r| {
                    Message::ReceiverStarted(gen, r)
                }),
                Task::perform(blocking(backend::list_local_ips), Message::LocalIps),
                Task::perform(blocking(backend::drop_settings), Message::Drop),
            ]),
            (Some(Role::Receive), Step::Done) => {
                // the last picked import folder comes back pre-selected
                Task::perform(blocking(|| backend::drop_settings().import_dir), Message::ImportFolder)
            }
            _ => Task::none(),
        }
    }

    fn discover(&mut self) -> Task<Message> {
        self.discover_run += 1;
        self.pair_status = PairStatus::Finding;
        let run = self.discover_run;
        Task::perform(blocking(backend::discover_peer), move |p| Message::Discovered(run, p))
    }

    /// Who "Pair" would connect to: the found peer, or the typed address.
    pub fn pair_target(&self) -> Option<Peer> {
        match self.pair_mode {
            PairMode::Manual => parse_addr(&self.addr_input),
            PairMode::Auto => self.peer.clone(),
        }
    }

    pub fn pair_ready(&self) -> bool {
        self.code_input.len() == 6 && self.pair_target().is_some() && self.pair_status != PairStatus::Pairing
    }

    fn submit_pair(&mut self) -> Task<Message> {
        if !self.pair_ready() {
            return Task::none();
        }
        let Some(peer) = self.pair_target() else { return Task::none() };
        let code = self.code_input.clone();
        self.pair_status = PairStatus::Pairing;
        self.pair_err.clear();
        if self.pair_mode == PairMode::Manual {
            self.peer = Some(peer.clone());
        }
        let gen = self.gen;
        Task::perform(
            blocking(move || {
                backend::pair(peer, code)?;
                Ok(backend::list_sources())
            }),
            move |r| Message::Paired(gen, r),
        )
    }

    fn on_backend(&mut self, ev: Event) -> Task<Message> {
        match ev {
            Event::Log => {
                self.log_rows = logbus::entries();
                if self.logs_open {
                    self.errors_seen = self.log_rows.last().map_or(0, |e| e.seq + 1);
                }
            }
            Event::Paired(peer) => {
                if self.role == Some(Role::Receive) && self.step == Step::Code {
                    ui_log("info", "ui::receive", &format!("sender {} ({}) connected", peer.name, peer.ip));
                    self.peer = Some(peer);
                    return self.go(Step::Transfer);
                }
            }
            Event::Progress(p) => {
                if self.role.is_none() {
                    return Task::none();
                }
                // log transitions only — ticks arrive several times a second
                if p.state != self.last_state {
                    self.last_state = p.state.clone();
                    let detail = if p.state == "error" {
                        format!(" — {}", p.error.as_deref().unwrap_or("no reason given"))
                    } else {
                        format!(" — {}/{} files", p.files_sent, p.files_total)
                    };
                    let level = if p.state == "error" { "error" } else { "info" };
                    ui_log(level, "ui::transfer", &format!("progress: {}{detail}", p.state));
                }
                let done = p.state == "done";
                if p.state == "error" {
                    self.stopping = false;
                }
                self.progress = p;
                if done {
                    return self.go(Step::Done);
                }
            }
        }
        Task::none()
    }

    /// Reset the UI. The caller also stops the backend session (unblocks the
    /// receiver's accept loop, ends transfers, stops the beacon).
    fn reset(&mut self) {
        let keep = (self.dark, self.gen);
        let logs = (
            self.logs_open,
            self.log_tall,
            self.log_floor,
            std::mem::take(&mut self.log_filter),
            std::mem::take(&mut self.log_rows),
            std::mem::take(&mut self.log_path),
            self.errors_seen,
        );
        *self = App::boot().0;
        self.dark = keep.0;
        self.gen = keep.1 + 1;
        (
            self.logs_open,
            self.log_tall,
            self.log_floor,
            self.log_filter,
            self.log_rows,
            self.log_path,
            self.errors_seen,
        ) = logs;
    }
}

fn idle_progress() -> TransferProgress {
    TransferProgress {
        state: "idle".into(),
        bytes_sent: 0,
        bytes_total: 0,
        files_sent: 0,
        files_total: 0,
        bytes_per_sec: 0.0,
        current_file: None,
        error: None,
    }
}

/// Backend events as a subscription. The channel is taken once; iced keeps
/// this stream alive for the whole run since the subscription never changes.
fn backend_events() -> impl iced::futures::Stream<Item = Message> {
    match backend::take_events() {
        Some(rx) => rx.map(Message::Backend).boxed(),
        None => iced::futures::stream::empty().boxed(),
    }
}

pub fn ui_log(level: &str, target: &str, msg: &str) {
    logbus::ui(level, target, msg);
}

/// Manual mode: build the peer from a typed address ("ip" or "ip:port").
pub fn parse_addr(input: &str) -> Option<Peer> {
    let t = input.trim();
    if t.is_empty() {
        return None;
    }
    let (ip, port) = match t.split_once(':') {
        Some((ip, p)) => (ip, p.parse::<u16>().ok()?),
        None => (t, TRANSFER_PORT),
    };
    let octets: Vec<&str> = ip.split('.').collect();
    let valid = octets.len() == 4
        && octets
            .iter()
            .all(|o| (1..=3).contains(&o.len()) && o.chars().all(|c| c.is_ascii_digit()));
    valid.then(|| Peer {
        name: ip.into(),
        ip: ip.into(),
        port,
    })
}

/// Windows paths: case-insensitive, either slash.
fn norm(p: &str) -> String {
    p.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn inside(child: &str, parent: &str) -> bool {
    norm(child).starts_with(&(norm(parent) + "\\"))
}

/// Add explorer picks without sending anything twice: skip exact repeats and
/// anything already covered by a picked folder; a new folder swallows earlier
/// picks that live inside it.
fn merge_picks(cur: &mut Vec<SourceGroup>, add: Vec<SourceGroup>) {
    for p in add {
        let path = p.path.clone().unwrap_or_default();
        let covered = cur.iter().any(|o| {
            let op = o.path.as_deref().unwrap_or("");
            norm(op) == norm(&path) || (o.kind == "folder" && inside(&path, op))
        });
        if covered {
            continue;
        }
        if p.kind == "folder" {
            cur.retain(|o| !inside(o.path.as_deref().unwrap_or(""), &path));
        }
        cur.push(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_parsing() {
        assert_eq!(parse_addr("192.168.1.24").map(|p| p.port), Some(TRANSFER_PORT));
        assert_eq!(parse_addr(" 169.254.42.9:50000 ").map(|p| p.port), Some(50000));
        assert!(parse_addr("192.168.1").is_none());
        assert!(parse_addr("pc.local").is_none());
        assert!(parse_addr("").is_none());
    }
}
