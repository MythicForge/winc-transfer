//! The step screens, one fn each — the terminal versions of `ui/steps.rs`.
//! Same flow and the same words where they apply off Windows too.

use super::app::{Msg, TuiApp, IN_ADDR, IN_CODE};
use super::kit::{
    accent, big_digits, bold, btn, copper, dim, fit_left, focus_style, good, ink, soft, width, BtnKind, Kind, Seg,
    Ui,
};
use crate::import::ImportEntry;
use crate::model::SourceGroup;
use crate::ui::format::{bytes, count, duration, eta, rate};
use crate::ui::{BrowserAction, Fw, PairMode, PairStatus, PickInto, Role, Step, TransferMode};
use ratatui::layout::Rect;
use ratatui::style::Style;
use std::path::Path;

pub fn body(app: &TuiApp, ui: &mut Ui) {
    match (app.role, app.step) {
        (None, _) => role_picker(ui),
        (_, Step::Connect) => connect(app, ui),
        (_, Step::Transfer) => transfer(app, ui),
        (_, Step::Done) => done(app, ui),
        (Some(Role::Send), Step::Pair) => pair(app, ui),
        (Some(Role::Send), Step::Select) => select(app, ui),
        (Some(Role::Receive), Step::Code) => code(app, ui),
        _ => {}
    }
}

/* ---------- role picker ---------- */

fn role_picker(ui: &mut Ui) {
    ui.gap();
    ui.line(&[("WIN", bold()), ("CI", copper().patch(bold())), ("  ·  PC-to-PC data crossing", dim())]);
    ui.gap();
    ui.line(&[("Move a whole PC across one wire.", bold())]);
    ui.text(
        "Connect the two computers with a Thunderbolt / USB4 cable — or use any Wi-Fi or Ethernet network \
         by entering the new PC's IP address. WINCI carries your files and browser data straight across — \
         no cloud, no accounts, end-to-end encrypted.",
        soft(),
    );
    ui.gap();
    ui.card(Some("On the OLD PC"), |ui| {
        ui.line(&[("Send my data", bold())]);
        ui.text(
            "This computer holds the files. Pick what to carry over and stream it to the new machine.",
            soft(),
        );
        ui.buttons(vec![btn("role:send", BtnKind::Primary, "Start sending →   s", Some(Msg::SetRole(Role::Send)))]);
    });
    ui.card(Some("On the NEW PC"), |ui| {
        ui.line(&[("Receive data", bold())]);
        ui.text(
            "This computer is the destination. Show a pairing code to the old PC and stand by.",
            soft(),
        );
        ui.buttons(vec![btn("role:recv", BtnKind::Plain, "Wait for a sender   r", Some(Msg::SetRole(Role::Receive)))]);
    });
    ui.autofocus = Some("role:send".into());
}

/* ---------- connect ---------- */

fn connect(app: &TuiApp, ui: &mut Ui) {
    let l = &app.link;
    let next = if app.role == Some(Role::Receive) { Step::Code } else { Step::Pair };
    ui.head(
        "Step 1 · Physical link",
        "Connect the cable — or use your network",
        "Join the two PCs with a Thunderbolt or USB4 cable — it comes up as a direct network link, no \
         router, no Wi-Fi. WINCI watches for it. No cable? Skip this step and connect over any Wi-Fi / \
         Ethernet network by IP address instead.",
    );
    ui.card(None, |ui| {
        if l.up {
            ui.status(
                false,
                copper(),
                &format!(
                    "Link up on {} · this PC is {}",
                    l.adapter.clone().unwrap_or_default(),
                    l.local_ip.clone().unwrap_or_default()
                ),
            );
        } else if let Some(a) = &l.adapter {
            ui.status(
                true,
                dim(),
                &format!(
                    "Cable detected on {a} — waiting for its 169.254.x.x address (DHCP timeout, up to ~30 s)…"
                ),
            );
        } else {
            ui.status(true, dim(), "Waiting for a direct cable link… (the address can take a few seconds)");
        }
        if !l.up {
            ui.gap();
            ui.bullets(&[
                "Use a cable rated for data (Thunderbolt 3/4 or USB4), not charge-only.",
                "On the other PC, launch WINCI too and choose the opposite role.",
                "The cable shows up as an Ethernet adapter with a 169.254.x.x address — that's normal.",
            ]);
        }
    });

    // the actions sit above the diagnostics: on a 24-row terminal they'd
    // otherwise start out of sight
    let skip = if app.role == Some(Role::Receive) {
        "No cable — receive over Wi-Fi / Ethernet →"
    } else {
        "No cable — connect by IP over Wi-Fi / Ethernet →"
    };
    ui.buttons(vec![
        btn("next", BtnKind::Primary, "Cable is connected →", l.up.then_some(Msg::GoTo(next))),
        btn("skip", BtnKind::Plain, skip, Some(Msg::GoTo(next))),
    ]);
    ui.gap();
    ui.card(Some("Detected adapters"), |ui| {
        if app.adapters.is_empty() {
            ui.line(&[("No IPv4 adapters found yet…", dim())]);
        }
        let name_w = app.adapters.iter().map(|a| width(&a.name)).max().unwrap_or(0).min(28) as u16;
        for a in &app.adapters {
            let tag = if a.link_local {
                "DIRECT CABLE"
            } else if a.cable {
                "CABLE"
            } else {
                "NETWORK"
            };
            let ip = if a.ip.is_empty() { "no IPv4 yet".to_string() } else { a.ip.clone() };
            let (x, y) = (ui.x, ui.y);
            ui.put(x, y, &a.name, if a.cable { bold() } else { ink() });
            let c = ui.put(x + name_w + 2, y, &format!("{ip:<16}"), soft());
            ui.put(x + name_w + 3 + c, y, tag, if a.cable { copper() } else { dim() });
            ui.gap();
        }
        let no_cable_adapter = !app.adapters.is_empty() && !app.adapters.iter().any(|a| a.cable);
        let cable_without_ip = app.adapters.iter().any(|a| a.cable && a.ip.is_empty());
        if !l.up && no_cable_adapter {
            ui.gap();
            ui.warn(if cfg!(windows) {
                "No cable adapter yet. If the cable is plugged in, wait a moment for its 169.254 address, or \
                 enable “Thunderbolt Networking” in Windows settings."
            } else {
                "No cable adapter yet. If the cable is plugged in, wait a moment for its 169.254 address, or \
                 check the thunderbolt-net driver is loaded (sudo modprobe thunderbolt-net)."
            });
        }
        if !l.up && cable_without_ip {
            ui.gap();
            ui.warn(if cfg!(windows) {
                "Cable adapter is up but has no IPv4 address yet. If it stays that way past a minute, give it one \
                 by hand: Settings → Network → the cable adapter → “Edit IP assignment” → Manual, IPv4 on — use \
                 169.254.1.1 on one PC and 169.254.1.2 on the other, mask 255.255.0.0, no gateway."
            } else {
                "Cable adapter is up but has no IPv4 address yet. If it stays that way past a minute, give it one \
                 by hand: sudo ip addr add 169.254.1.1/16 dev <adapter> on one PC and 169.254.1.2/16 on the \
                 other."
            });
        }
        ui.gap();
        ui.buttons(vec![btn("refresh", BtnKind::Ghost, "↻ Refresh", Some(Msg::PollAdapters))]);
        if cfg!(windows) {
            let fw_label = match app.fw {
                Fw::Working => "Waiting for admin approval…",
                Fw::Done => "✓ Firewall rule added",
                _ => "Allow WINCI through the firewall",
            };
            ui.gap();
            ui.text(
                "If the PCs never find each other, Windows Firewall is usually blocking WINCI on “Public” \
                 networks — the cable link counts as one.",
                soft(),
            );
            ui.buttons(vec![btn(
                "firewall",
                BtnKind::Plain,
                fw_label,
                (app.fw != Fw::Working).then_some(Msg::AllowFirewall),
            )]);
            if app.fw == Fw::Error {
                ui.warn(
                    "Couldn’t add the rule — the admin prompt was declined or blocked. You can also allow WINCI \
                     manually in “Windows Security → Firewall → Allow an app”.",
                );
            }
        } else {
            ui.gap();
            ui.text(
                &format!(
                    "If the PCs never find each other, a firewall is usually blocking WINCI: allow UDP {} \
                     (discovery) and TCP {} (transfer) in.",
                    crate::model::DISCOVERY_PORT,
                    crate::model::TRANSFER_PORT
                ),
                soft(),
            );
        }
    });

    ui.autofocus = Some(if l.up { "next" } else { "skip" }.into());
}

/* ---------- pair (sender) ---------- */

/// The 6-digit code as slots: 4 1 8 · 6 _ _
fn code_slots(ui: &mut Ui, digits: &str, enabled: bool) {
    let mut s = String::new();
    let chars: Vec<char> = digits.chars().collect();
    for i in 0..6 {
        if i == 3 {
            s.push_str("· ");
        }
        s.push(chars.get(i).copied().unwrap_or('_'));
        s.push(' ');
    }
    let label = format!(" {} ", s.trim_end());
    let focused = enabled && ui.focused(IN_CODE);
    let st = match (enabled, focused) {
        (false, _) => dim(),
        (true, true) => accent().patch(bold()).patch(focus_style()),
        (true, false) => accent().patch(bold()),
    };
    let (x, y) = (ui.x, ui.y);
    let mut w = ui.put(x, y, "› ", if focused { accent() } else { dim() });
    w += ui.put(x + w, y, &label, st);
    ui.hit(IN_CODE, Rect::new(x, y, w, 1), Msg::SubmitPair, Kind::Input);
    ui.gap();
}

fn pair(app: &TuiApp, ui: &mut Ui) {
    let pairing = app.pair_status == PairStatus::Pairing;
    ui.head(
        "Step 2 · Handshake",
        "Pair with the new PC",
        "Confirm the two machines with the 6-digit code shown on the new PC. Auto-find works over the direct \
         cable; use “Enter IP address” for Wi-Fi or Ethernet.",
    );
    ui.seg(vec![
        Seg {
            key: "mode:auto",
            label: "Find automatically",
            on: app.pair_mode == PairMode::Auto,
            msg: Msg::SetPairMode(PairMode::Auto),
        },
        Seg {
            key: "mode:ip",
            label: "Enter IP address",
            on: app.pair_mode == PairMode::Manual,
            msg: Msg::SetPairMode(PairMode::Manual),
        },
    ]);
    ui.gap();
    match app.pair_mode {
        PairMode::Auto => ui.card(None, |ui| {
            if app.pair_status == PairStatus::Finding {
                ui.status(true, dim(), "Looking for the new PC on the cable…");
            } else if let Some(peer) = &app.peer {
                ui.status(false, copper(), &format!("Found {} at {}", peer.name, peer.ip));
            } else {
                ui.status(false, dim(), "No peer yet.");
            }
        }),
        PairMode::Manual => {
            ui.eyebrow("New PC address (shown on its screen)");
            ui.input(
                IN_ADDR,
                &app.addr_edit,
                &format!("192.168.1.24  ·  or  169.254.42.9:{}", crate::model::TRANSFER_PORT),
                Msg::AddrSubmit,
                48,
            );
            if !app.addr_edit.text.is_empty() && app.pair_target().is_none() {
                ui.warn("Enter a valid IPv4 address, e.g. 192.168.1.24");
            }
        }
    }
    ui.gap();
    ui.eyebrow("6-digit code");
    code_slots(ui, &app.code_edit.text, !pairing);
    if app.pair_status == PairStatus::Error && !app.pair_err.is_empty() {
        ui.gap();
        ui.warn(&app.pair_err);
    }
    ui.gap();
    ui.buttons(vec![
        btn("back", BtnKind::Ghost, "← Back", Some(Msg::GoTo(Step::Connect))),
        btn(
            "pair",
            BtnKind::Primary,
            if pairing { "Pairing…" } else { "Pair →" },
            app.pair_ready().then_some(Msg::SubmitPair),
        ),
    ]);
    ui.autofocus = Some(
        if app.pair_mode == PairMode::Manual && app.pair_target().is_none() { IN_ADDR } else { IN_CODE }.into(),
    );
}

/* ---------- code (receiver) ---------- */

fn code(app: &TuiApp, ui: &mut Ui) {
    let port = app.self_peer.as_ref().map_or("—".into(), |s| s.port.to_string());
    ui.head(
        "Step 2 · Handshake",
        "Read this code to the old PC",
        "On the old computer, WINCI is asking for a 6-digit code. Type the numbers below over there to confirm \
         the two machines are paired.",
    );
    ui.card(None, |ui| {
        if app.code.is_empty() {
            ui.line(&[("··· ···", copper().patch(bold()))]);
        } else if let Some(rows) = big_digits(&app.code) {
            for r in rows {
                ui.line(&[(&r, copper().patch(bold()))]);
            }
            ui.line(&[(&app.code, copper().patch(bold()))]);
        } else {
            ui.line(&[(&app.code, copper().patch(bold()))]);
        }
        ui.text("Enter this on the old PC to pair", soft());
    });
    if let Some(e) = &app.receive_err {
        ui.warn(&format!("Couldn’t start receiving — {e}"));
    }
    ui.card(Some("No cable? Pair by IP address"), |ui| {
        ui.text(
            &format!(
                "If the old PC can’t find this one automatically, choose “Enter IP address” over there and type \
                 one of these addresses. Port is {port}."
            ),
            soft(),
        );
        if app.ips.is_empty() {
            ui.line(&[("Reading addresses…", dim())]);
        } else {
            let mut segs: Vec<(String, Style)> = Vec::new();
            for ip in &app.ips {
                // the cable's link-local address, if there is one
                let direct = ip.starts_with("169.254.");
                segs.push((ip.clone(), bold()));
                segs.push((if direct { " (direct link)   ".into() } else { "   ".into() }, copper()));
            }
            let refs: Vec<(&str, Style)> = segs.iter().map(|(s, st)| (s.as_str(), *st)).collect();
            ui.rich(&refs);
        }
    });
    ui.card(Some("Where incoming data lands"), |ui| {
        let landing = app.drop.as_ref().map_or("…".into(), |d| {
            Path::new(&d.receive_root).join("WINC Received").display().to_string()
        });
        let free = app.drop_free.map(|f| format!(" — {} free.", bytes(f as f64))).unwrap_or_default();
        ui.rich(&[("Received files go to ", soft()), (&landing, bold()), (&free, soft())]);
        ui.text(
            "If this drive is short on space, pick a bigger one before the old PC connects. WINCI remembers the \
             choice.",
            dim(),
        );
        let mut b = Vec::new();
        if app.drop.as_ref().is_some_and(|d| !d.receive_is_default) {
            b.push(btn("drop:reset", BtnKind::Ghost, "Use the default folder", Some(Msg::ResetDrop)));
        }
        b.push(btn("drop:change", BtnKind::Plain, "Change folder…", Some(Msg::ChangeDrop)));
        ui.buttons(b);
    });
    ui.gap();
    ui.status(true, dim(), "Listening for the old PC to connect…");
}

/* ---------- select (sender) ---------- */

/// Size and item count, right-aligned on the two lines of a row.
fn meta(s: &SourceGroup) -> (String, String) {
    let items = if s.kind == "file" { "file".to_string() } else { format!("{} items", count(s.items)) };
    (bytes(s.bytes as f64), items)
}

/// A two-line source row: [x] Label … size / hint … items. Clickable as a whole.
fn source_row(ui: &mut Ui, s: &SourceGroup) {
    let key = format!("src:{}", s.id);
    let focused = ui.focused(&key);
    let (x, y) = (ui.x, ui.y);
    let (size, items) = meta(s);
    let right = x + ui.w;
    let box_st = if s.selected { accent().patch(bold()) } else { dim() };
    ui.put(x, y, if focused { "▸" } else { " " }, accent());
    ui.put(x + 2, y, if s.selected { "[✓]" } else { "[ ]" }, box_st);
    let label_st = if focused { bold().patch(focus_style()) } else { bold() };
    let sw = width(&size) as u16;
    let iw = width(&items) as u16;
    let badge = if s.kind == "browser" { 9 } else { 0 };
    let room = ui.w.saturating_sub(6 + sw + badge + 2);
    let c = ui.put_n(x + 6, y, &format!(" {} ", s.label), label_st, room);
    if s.kind == "browser" {
        ui.put(x + 7 + c, y, "BROWSER", copper());
    }
    ui.put(right.saturating_sub(sw), y, &size, ink());
    let room = ui.w.saturating_sub(7 + iw + 2);
    ui.put(x + 7, y + 1, &fit_left(&s.hint, room as usize), dim());
    ui.put(right.saturating_sub(iw), y + 1, &items, soft());
    ui.hit(&key, Rect::new(x, y, ui.w, 2), Msg::Toggle(s.id.clone()), Kind::Toggle);
    ui.y += 2;
    if let (true, Some(cv)) = (s.selected, &s.caveat) {
        let (ox, ow) = (ui.x, ui.w);
        ui.x += 7;
        ui.w = ui.w.saturating_sub(7).max(1);
        ui.warn(cv);
        ui.x = ox;
        ui.w = ow;
    }
}

/// A Custom Transfer pick: ▣ name … size [✕]
fn pick_row(ui: &mut Ui, s: &SourceGroup) {
    let (x, y) = (ui.x, ui.y);
    let (size, items) = meta(s);
    let right = x + ui.w;
    let rm = btn(&format!("rm:{}", s.id), BtnKind::Ghost, "✕", Some(Msg::RemovePick(s.id.clone())));
    let rm_w = 5;
    let sw = width(&size) as u16;
    let iw = width(&items) as u16;
    ui.put(x + 2, y, if s.kind == "file" { "▤" } else { "▣" }, accent());
    ui.put_n(x + 4, y, &s.label, bold(), ui.w.saturating_sub(4 + sw + rm_w + 3));
    ui.put(right.saturating_sub(sw + rm_w + 1), y, &size, ink());
    ui.button_at(right.saturating_sub(rm_w), y, &rm);
    let room = ui.w.saturating_sub(4 + iw + rm_w + 3);
    ui.put(x + 4, y + 1, &fit_left(&s.hint, room as usize), dim());
    ui.put(right.saturating_sub(iw + rm_w + 1), y + 1, &items, soft());
    ui.y += 2;
}

fn select(app: &TuiApp, ui: &mut Ui) {
    let custom = app.mode == TransferMode::Custom;
    ui.head(
        "Step 3 · Payload",
        "Choose what to carry over",
        if custom {
            "Pick exactly what to send. Everything in the list crosses the cable."
        } else {
            "WINCI found these on the old PC. Tick what should land on the new machine."
        },
    );
    ui.seg(vec![
        Seg {
            key: "tm:user",
            label: "User Transfer",
            on: !custom,
            msg: Msg::SetMode(TransferMode::User),
        },
        Seg {
            key: "tm:custom",
            label: "Custom Transfer",
            on: custom,
            msg: Msg::SetMode(TransferMode::Custom),
        },
    ]);
    ui.text(
        if custom {
            "Pick any files and folders on this PC"
        } else {
            "Your folders & browser data, found for you"
        },
        dim(),
    );
    ui.gap();

    let first_key = if custom {
        if app.picks.is_empty() {
            ui.card(None, |ui| {
                ui.text(
                    "Nothing picked yet. Add folders or files from anywhere on this PC — other drives included. \
                     They land in Documents on the new PC.",
                    soft(),
                );
            });
        }
        for s in &app.picks {
            pick_row(ui, s);
        }
        let mut b = vec![btn(
            "add:picks",
            BtnKind::Plain,
            "+ Add files / folders…",
            (!app.picking).then_some(Msg::Pick(PickInto::Picks)),
        )];
        if !app.picks.is_empty() {
            b.push(btn("clear", BtnKind::Ghost, "Clear list", Some(Msg::ClearPicks)));
        }
        ui.buttons(b);
        "add:picks".to_string()
    } else {
        if app.sources.is_empty() {
            ui.line(&[("No user folders or browsers found on this PC — use Custom Transfer, or add a folder.", dim())]);
        }
        for s in &app.sources {
            source_row(ui, s);
        }
        let all_on = !app.sources.is_empty() && app.sources.iter().all(|g| g.selected);
        ui.buttons(vec![
            btn(
                "add:src",
                BtnKind::Plain,
                "+ Add a folder…",
                (!app.picking).then_some(Msg::Pick(PickInto::Sources)),
            ),
            btn(
                "all",
                BtnKind::Ghost,
                if all_on { "Deselect all" } else { "Select all" },
                (!app.sources.is_empty()).then_some(Msg::SelectAll(!all_on)),
            ),
        ]);
        app.sources.first().map_or("add:src".into(), |s| format!("src:{}", s.id))
    };
    if app.picking {
        ui.line(&[("Measuring…", dim())]);
    }

    let chosen: Vec<&SourceGroup> = if custom {
        app.picks.iter().collect()
    } else {
        app.sources.iter().filter(|g| g.selected).collect()
    };
    let total_b: u64 = chosen.iter().map(|s| s.bytes).sum();
    let total_n: u64 = chosen.iter().map(|s| s.items).sum();
    ui.gap();
    ui.rule();
    ui.line(&[(&bytes(total_b as f64), bold()), (&format!(" across {} items", count(total_n)), soft())]);
    ui.buttons(vec![
        btn(
            "start",
            BtnKind::Primary,
            "Start transfer →",
            (!chosen.is_empty() && !app.picking && !app.testing).then_some(Msg::StartTransfer),
        ),
        btn(
            "bench",
            BtnKind::Plain,
            if app.testing { "Testing link…" } else { "Test link speed" },
            (!app.testing).then_some(Msg::RunBench),
        ),
    ]);
    if !app.speed.is_empty() {
        ui.text(&app.speed, soft());
    }
    ui.autofocus = Some(first_key);
}

/* ---------- transfer ---------- */

fn transfer(app: &TuiApp, ui: &mut Ui) {
    let g = &app.progress;
    let sending = app.role == Some(Role::Send);
    let running = g.state == "running" || g.state == "reconnecting";
    let pct = if g.bytes_total > 0 {
        (g.bytes_sent as f64 / g.bytes_total as f64 * 100.0).min(100.0)
    } else {
        0.0
    };
    ui.head(
        &format!("Step {} · Crossing", if sending { 4 } else { 3 }),
        if sending { "Sending to the new PC" } else { "Receiving from the old PC" },
        "Keep the cable connected. Data moves directly between the two machines — nothing touches the internet.",
    );

    let meters = [
        ("Throughput", rate(g.bytes_per_sec), String::new()),
        ("Moved", bytes(g.bytes_sent as f64), format!(" / {}", bytes(g.bytes_total as f64))),
        ("Items", count(g.files_sent), format!(" / {}", count(g.files_total))),
        (
            "Time left",
            eta(g.bytes_total.saturating_sub(g.bytes_sent) as f64, g.bytes_per_sec),
            String::new(),
        ),
    ];
    let cols: u16 = if ui.w >= 76 { 4 } else { 2 };
    let cw = ui.w / cols;
    for chunk in meters.chunks(cols as usize) {
        let (x, y) = (ui.x, ui.y);
        for (i, (k, v, of)) in chunk.iter().enumerate() {
            let cx = x + cw * i as u16;
            ui.put(cx, y, &k.to_uppercase(), dim());
            let c = ui.put(cx, y + 1, v, bold());
            ui.put(cx + c, y + 1, of, dim());
        }
        ui.y += 3;
    }
    ui.bar(pct);
    if g.state == "reconnecting" {
        ui.gap();
        ui.warn(
            "The connection was interrupted or data arrived damaged. WINCI is reconnecting and will pick up from \
             the file that was in flight — nothing already received is sent again.",
        );
    }
    let now_file = match (&g.current_file, g.state.as_str()) {
        (Some(f), _) => f.clone(),
        (None, "idle") => "Preparing…".into(),
        _ => String::new(),
    };
    ui.line(&[(&now_file, soft())]);
    if g.state == "error" {
        ui.gap();
        ui.warn(&g.error.clone().unwrap_or_else(|| "Transfer stopped.".into()));
    }
    ui.gap();
    if running || g.state == "idle" {
        ui.buttons(vec![btn(
            "stop",
            BtnKind::Plain,
            if app.stopping { "Stopping…" } else { "Stop transfer" },
            (!app.stopping).then_some(Msg::Stop),
        )]);
        ui.autofocus = Some("stop".into());
    } else {
        ui.buttons(vec![btn("again", BtnKind::Ghost, "← Start over", Some(Msg::Reset))]);
        ui.autofocus = Some("again".into());
    }
}

/* ---------- done ---------- */

fn badge_for(action: &str) -> (&'static str, Style) {
    match action {
        "imported" => ("✓ IMPORTED", good()),
        "skipped-not-fresh" => ("SKIPPED", copper()),
        "skipped-not-installed" => ("NOT INSTALLED", copper()),
        _ => ("FAILED", super::kit::bad()),
    }
}

fn import_row(app: &TuiApp, ui: &mut Ui, en: &ImportEntry) {
    let busy = app.pending_browser.is_some();
    let mine = busy && app.pending_browser.as_deref() == en.browser_label.as_deref();
    let (x, y) = (ui.x, ui.y);
    ui.put(x, y, &en.label, bold());
    let mut cx = x + 22;
    match (en.action.as_str(), en.browser_label.clone()) {
        ("skipped-not-fresh", Some(bl)) => {
            let b = btn(
                &format!("br:{bl}"),
                BtnKind::Danger,
                if mine { "Overwriting…" } else { "Overwrite?" },
                (!busy).then_some(Msg::Browser(bl, BrowserAction::Overwrite)),
            );
            cx += ui.button_at(cx, y, &b);
        }
        ("skipped-not-installed", Some(bl)) => {
            let b = btn(
                &format!("br:{bl}"),
                BtnKind::Danger,
                if mine { "Checking…" } else { "Open First" },
                (!busy).then_some(Msg::Browser(bl, BrowserAction::Retry)),
            );
            cx += ui.button_at(cx, y, &b);
        }
        (action, _) => {
            let (label, st) = badge_for(action);
            cx += ui.put(cx, y, label, st);
        }
    }
    if en.count > 0 {
        let files = if en.count == 1 { "1 file".to_string() } else { format!("{} files", count(en.count)) };
        cx += ui.put(cx + 2, y, &files, soft()) + 2;
    }
    let _ = cx;
    ui.y += 1;
    if let Some(d) = &en.detail {
        let (ox, ow) = (ui.x, ui.w);
        ui.x += 2;
        ui.w = ui.w.saturating_sub(2).max(1);
        ui.text(d, dim());
        ui.x = ox;
        ui.w = ow;
    }
}

fn import_card(app: &TuiApp, ui: &mut Ui) {
    ui.card(Some("Import into place"), |ui| match &app.report {
        None => {
            let landed = app.received_dir.clone().unwrap_or_else(|| "WINC Received".into());
            ui.rich(&[("Files landed in ", soft()), (&landed, bold()), (".", soft())]);
            ui.text(
                "Import moves them into your real folders — or under a folder you choose — and nothing there is \
                 overwritten; incoming duplicates are kept as “name (from old PC)”. Browser data imports only \
                 into browsers with no existing data; ones that already have data show an “Overwrite?” option, \
                 and ones not yet opened show “Open First”.",
                soft(),
            );
            ui.seg(vec![
                Seg {
                    key: "imp:user",
                    label: "Your user folders",
                    on: !app.use_folder,
                    msg: Msg::UseImportFolder(false),
                },
                Seg {
                    key: "imp:folder",
                    label: "A folder you choose",
                    on: app.use_folder,
                    msg: Msg::UseImportFolder(true),
                },
            ]);
            if app.use_folder {
                let folder = app.import_folder.clone().unwrap_or_else(|| "No folder chosen yet".into());
                let free = app.folder_free.map(|f| format!("  {} free", bytes(f as f64))).unwrap_or_default();
                ui.rich(&[(&folder, bold()), (&free, dim())]);
                ui.buttons(vec![btn("imp:choose", BtnKind::Plain, "Choose folder…", Some(Msg::ChooseImportFolder))]);
                ui.text(
                    "Documents, Desktop, Pictures… land as subfolders of it; custom picks go straight in. \
                     Remembered for next time.",
                    dim(),
                );
            }
            let can = !app.importing && app.received_dir.is_some() && !(app.use_folder && app.import_folder.is_none());
            ui.gap();
            ui.buttons(vec![btn(
                "import",
                BtnKind::Primary,
                if app.importing { "Importing…" } else { "Import into place" },
                can.then_some(Msg::RunImport),
            )]);
            if let Some(e) = &app.import_err {
                ui.warn(&format!("Import failed — {e}"));
            }
        }
        Some(r) => {
            if r.entries.is_empty() {
                ui.text("Nothing to import.", soft());
            }
            for en in &r.entries {
                import_row(app, ui, en);
            }
            ui.gap();
            ui.text(
                "A snapshot log of every file (source → destination) was saved as import-log-*.json in the WINC \
                 Received folder.",
                dim(),
            );
            if let Some(e) = &app.import_err {
                ui.warn(e);
            }
        }
    });
}

fn done(app: &TuiApp, ui: &mut Ui) {
    let sending = app.role == Some(Role::Send);
    let g = &app.progress;
    ui.gap();
    ui.line(&[
        ("✓  ", good().patch(bold())),
        (if sending { "Everything sent" } else { "Everything landed" }, bold()),
    ]);
    ui.text(
        &format!("{} across {} items crossed the cable.", bytes(g.bytes_total as f64), count(g.files_total)),
        soft(),
    );
    if let Some(t) = app.transfer_took {
        let secs = t.as_secs_f64().max(0.001);
        ui.line(&[(&format!("Took {} · average {}", duration(secs), rate(g.bytes_total as f64 / secs)), ink())]);
    }
    ui.gap();
    ui.card(Some("What next"), |ui| {
        ui.bullets(&[
            "Open the new PC and check a few folders landed where you expect.",
            "Sign into your browser to finish syncing bookmarks and history.",
            "Saved passwords only unlock under the same Windows / Microsoft account.",
            "You can now safely unplug the cable.",
        ]);
    });
    if !sending {
        import_card(app, ui);
    }
    ui.gap();
    ui.buttons(vec![btn("again", BtnKind::Plain, "Transfer something else", Some(Msg::Reset))]);
    let can_import = !sending && app.report.is_none() && app.received_dir.is_some();
    ui.autofocus = Some(if can_import { "import" } else { "again" }.into());
}
