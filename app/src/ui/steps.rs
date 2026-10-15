//! The step screens, one fn each.

use super::format::{bytes, count, eta, rate};
use super::style::{self, Btn, Pal, MONO};
use super::widgets::{body, eyebrow, mono, panel_head, spinner, warn_line};
use super::{
    App, BrowserAction, Fw, Message, PairMode, PairStatus, PickInto, Role, Step, TransferMode, CODE_INPUT,
};
use crate::import::ImportEntry;
use crate::model::{SourceGroup, TRANSFER_PORT};
use iced::widget::{button, column, container, progress_bar, row, space, text, text_input, Column};
use iced::{Alignment, Element, Fill, Padding};

pub fn view(app: &App) -> Element<'_, Message> {
    match (app.role, app.step) {
        (_, Step::Connect) => connect(app),
        (_, Step::Transfer) => transfer(app),
        (_, Step::Done) => done(app),
        (Some(Role::Send), Step::Pair) => pair(app),
        (Some(Role::Send), Step::Select) => select(app),
        (Some(Role::Receive), Step::Code) => code(app),
        _ => space().into(),
    }
}

/* ---------- shared bits ---------- */

fn btn<'a>(p: Pal, kind: Btn, label: impl text::IntoFragment<'a>, on: Option<Message>) -> button::Button<'a, Message> {
    button(text(label))
        .on_press_maybe(on)
        .style(style::btn(p, kind))
        .padding([9, 16])
}

fn card<'a>(p: Pal, content: impl Into<Element<'a, Message>>) -> container::Container<'a, Message> {
    container(content).padding([18, 20]).width(Fill).style(style::card(p))
}

/// Spinner (or a status dot) + a line of text.
fn waiting<'a>(app: &App, busy: bool, dot: iced::Color, line: Element<'a, Message>) -> Element<'a, Message> {
    let lead: Element<_> = if busy {
        spinner(app.pal(), app.phase())
    } else {
        container(space().width(8).height(8)).style(style::tinted(dot, dot, 4.0)).into()
    };
    row![lead, line].spacing(12).align_y(Alignment::Center).into()
}

fn bullets<'a>(p: Pal, items: &[&'a str]) -> Element<'a, Message> {
    Column::with_children(items.iter().map(|s| {
        row![body("•", p.ink_2), body(*s, p.ink_2)].spacing(8).into()
    }))
    .spacing(6)
    .into()
}

fn actions<'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    row(items).spacing(10).padding(Padding::ZERO.top(22)).into()
}

/* ---------- connect ---------- */

fn connect(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let l = &app.link;
    let next = if app.role == Some(Role::Receive) { Step::Code } else { Step::Pair };

    let status: Element<_> = if l.up {
        waiting(
            app,
            false,
            p.copper,
            body(
                format!(
                    "Link up on {} · this PC is {}",
                    l.adapter.clone().unwrap_or_default(),
                    l.local_ip.clone().unwrap_or_default()
                ),
                p.ink,
            )
            .into(),
        )
    } else if let Some(a) = &l.adapter {
        // cable adapter is up, but Windows hasn't assigned its IPv4 yet
        waiting(
            app,
            true,
            p.slate,
            body(
                format!(
                    "Cable detected on {a} — waiting for Windows to assign its 169.254.x.x address \
                     (DHCP timeout, up to ~30 s)…"
                ),
                p.ink,
            )
            .into(),
        )
    } else {
        waiting(
            app,
            true,
            p.slate,
            body("Waiting for a direct cable link… (the address can take a few seconds)", p.ink).into(),
        )
    };
    let mut link_card = column![status].spacing(14);
    if !l.up {
        link_card = link_card.push(bullets(
            p,
            &[
                "Use a cable rated for data (Thunderbolt 3/4 or USB4), not charge-only.",
                "On the other PC, launch WINCI too and choose the opposite role.",
                "The cable shows up as an Ethernet adapter with a 169.254.x.x address — that's normal.",
            ],
        ));
    }

    // live diagnostics: what the app actually sees
    let mut list = column![].spacing(6);
    if app.adapters.is_empty() {
        list = list.push(mono("No IPv4 adapters found yet…", style::S_SM, p.slate));
    }
    for a in &app.adapters {
        let tag = if a.link_local {
            "direct cable"
        } else if a.cable {
            "cable"
        } else {
            "network"
        };
        let tag_c = if a.cable { p.copper } else { p.slate };
        list = list.push(
            container(
                row![
                    text(a.name.clone()).width(Fill),
                    mono(
                        if a.ip.is_empty() { "no IPv4 yet".to_string() } else { a.ip.clone() },
                        style::S_SM,
                        p.ink_2
                    ),
                    container(mono(tag.to_uppercase(), 10.0, tag_c)).padding([1, 6]).style(style::badge(tag_c)),
                ]
                .spacing(14)
                .align_y(Alignment::Center),
            )
            .padding([8, 12])
            .style(style::row_box(p, a.cable)),
        );
    }
    let no_cable_adapter = !app.adapters.is_empty() && !app.adapters.iter().any(|a| a.cable);
    let cable_without_ip = app.adapters.iter().any(|a| a.cable && a.ip.is_empty());

    let mut diag = column![
        row![
            eyebrow(p, "Detected adapters"),
            space().width(Fill),
            btn(p, Btn::Ghost, "↻ Refresh", Some(Message::PollAdapters)),
        ]
        .align_y(Alignment::Center),
        list,
    ]
    .spacing(10);
    if !l.up && no_cable_adapter {
        diag = diag.push(warn_line(
            p,
            "No cable adapter yet. If the cable is plugged in, wait a moment for its 169.254 address, \
             or enable “Thunderbolt Networking” in Windows settings.",
        ));
    }
    if !l.up && cable_without_ip {
        diag = diag.push(warn_line(
            p,
            "Cable adapter is up but has no IPv4 address yet. If it stays that way past a minute, give it \
             one by hand: Settings → Network → the cable adapter → “Edit IP assignment” → Manual, IPv4 on — \
             use 169.254.1.1 on one PC and 169.254.1.2 on the other, mask 255.255.0.0, no gateway.",
        ));
    }

    // Windows treats the cable link as an "Unidentified network" (Public
    // profile) and silently blocks WINC's discovery + listener there — the
    // standard firewall prompt only covers Private networks.
    let fw_label = match app.fw {
        Fw::Working => "Waiting for admin approval…",
        Fw::Done => "✓ Firewall rule added",
        _ => "Allow WINCI through the firewall",
    };
    diag = diag.push(space().height(4));
    diag = diag.push(body(
        "If the PCs never find each other, Windows Firewall is usually blocking WINCI on “Public” \
         networks — the cable link counts as one.",
        p.ink_2,
    ));
    diag = diag.push(btn(
        p,
        Btn::Plain,
        fw_label,
        (app.fw != Fw::Working).then_some(Message::AllowFirewall),
    ));
    if app.fw == Fw::Error {
        diag = diag.push(warn_line(
            p,
            "Couldn’t add the rule — the admin prompt was declined or blocked. You can also allow WINCI \
             manually in “Windows Security → Firewall → Allow an app”.",
        ));
    }

    let skip = if app.role == Some(Role::Receive) {
        "No cable — receive over Wi-Fi / Ethernet →"
    } else {
        "No cable — connect by IP over Wi-Fi / Ethernet →"
    };
    column![
        panel_head(
            p,
            "Step 1 · Physical link",
            "Connect the cable — or use your network",
            "Join the two PCs with a Thunderbolt or USB4 cable. Windows brings up a direct network bridge \
             automatically — no router, no Wi-Fi. WINCI watches for the link. No cable? Skip this step and \
             connect over any Wi-Fi / Ethernet network by IP address instead.",
        ),
        card(p, link_card).padding(22),
        card(p, diag),
        actions(vec![
            btn(p, Btn::Primary, "Cable is connected →", l.up.then_some(Message::GoTo(next))).into(),
            btn(p, Btn::Ghost, skip, Some(Message::GoTo(next))).into(),
        ]),
    ]
    .spacing(18)
    .into()
}

/* ---------- pair (sender) ---------- */

fn segmented<'a>(p: Pal, items: Vec<(&'a str, bool, Message)>) -> Element<'a, Message> {
    container(
        row(items.into_iter().map(|(label, on, msg)| {
            button(text(label).font(style::SEMI))
                .on_press(msg)
                .style(style::seg(p, on))
                .padding([8, 16])
                .into()
        }))
        .spacing(3),
    )
    .padding(3)
    .style(style::tinted(p.paper_2, p.hairline, style::RADIUS_SM))
    .into()
}

fn field_label<'a>(p: Pal, s: &'a str) -> Element<'a, Message> {
    mono(s.to_uppercase(), style::S_SM, p.slate).into()
}

fn pair(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let pairing = app.pair_status == PairStatus::Pairing;

    let target: Element<_> = match app.pair_mode {
        PairMode::Auto => {
            let line: Element<_> = if app.pair_status == PairStatus::Finding {
                waiting(app, true, p.slate, body("Looking for the new PC on the cable…", p.ink).into())
            } else if let Some(peer) = &app.peer {
                waiting(
                    app,
                    false,
                    p.copper,
                    body(format!("Found {} at {}", peer.name, peer.ip), p.ink).into(),
                )
            } else {
                waiting(app, false, p.slate, body("No peer yet.", p.ink).into())
            };
            card(p, line).padding(22).into()
        }
        PairMode::Manual => {
            let mut c = column![
                field_label(p, "New PC address (shown on its screen)"),
                text_input(&format!("192.168.1.24   ·   or  169.254.42.9:{TRANSFER_PORT}"), &app.addr_input)
                    .on_input(Message::AddrInput)
                    .on_submit(Message::AddrSubmit)
                    .font(MONO)
                    .size(style::S_1)
                    .padding([14, 16])
                    .style(style::input(p)),
            ]
            .spacing(8);
            if !app.addr_input.is_empty() && app.pair_target().is_none() {
                c = c.push(warn_line(p, "Enter a valid IPv4 address, e.g. 192.168.1.24"));
            }
            c.into()
        }
    };

    let mut code_input = text_input("000000", &app.code_input)
        .id(CODE_INPUT)
        .font(MONO)
        .size(34)
        .padding(18)
        .align_x(Alignment::Center)
        .style(style::input(p));
    if !pairing {
        code_input = code_input.on_input(Message::CodeInput).on_submit(Message::SubmitPair);
    }

    let mut col = column![
        panel_head(
            p,
            "Step 2 · Handshake",
            "Pair with the new PC",
            "Confirm the two machines with the 6-digit code shown on the new PC. Auto-find works over the \
             direct cable; use “Enter IP” for Wi-Fi or Ethernet.",
        ),
        segmented(
            p,
            vec![
                ("Find automatically", app.pair_mode == PairMode::Auto, Message::SetPairMode(PairMode::Auto)),
                ("Enter IP address", app.pair_mode == PairMode::Manual, Message::SetPairMode(PairMode::Manual)),
            ]
        ),
        target,
        field_label(p, "6-digit code"),
        code_input,
    ]
    .spacing(14);
    if app.pair_status == PairStatus::Error && !app.pair_err.is_empty() {
        col = col.push(warn_line(p, app.pair_err.clone()));
    }
    col.push(actions(vec![
        btn(p, Btn::Ghost, "← Back", Some(Message::GoTo(Step::Connect))).into(),
        btn(
            p,
            Btn::Primary,
            if pairing { "Pairing…" } else { "Pair →" },
            app.pair_ready().then_some(Message::SubmitPair),
        )
        .into(),
    ]))
    .into()
}

/* ---------- code (receiver) ---------- */

fn code(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let digits = if app.code.is_empty() { "··· ···".to_string() } else { app.code.clone() };
    let port = app.self_peer.as_ref().map_or("—".into(), |s| s.port.to_string());

    let codebox = container(
        column![
            text(digits).font(MONO).size(54).style(style::color(p.ink)),
            body("Enter this on the old PC to pair", p.ink_2),
        ]
        .spacing(8)
        .align_x(Alignment::Center),
    )
    .padding(28)
    .center_x(Fill)
    .style(style::tinted(p.copper_soft, p.copper, style::RADIUS));

    let chips: Element<_> = if app.ips.is_empty() {
        mono("Reading addresses…", style::S_SM, p.slate).into()
    } else {
        row(app.ips.iter().enumerate().map(|(i, ip)| {
            let mut r = row![mono(ip.clone(), style::S_0, p.ink)].spacing(8).align_y(Alignment::Center);
            if i == 0 {
                r = r.push(text("direct link").size(style::S_SM).style(style::color(p.copper)));
            }
            container(r)
                .padding([6, 12])
                .style(style::tinted(
                    if i == 0 { p.copper_soft } else { p.panel },
                    if i == 0 { p.copper } else { p.hairline },
                    100.0,
                ))
                .into()
        }))
        .spacing(8)
        .wrap()
        .into()
    };

    let landing = app.drop.as_ref().map_or("…".into(), |d| {
        format!("{}\\WINC Received", d.receive_root.trim_end_matches('\\'))
    });
    let free = app.drop_free.map(|f| format!(" — {} free.", bytes(f as f64))).unwrap_or_default();
    let mut drop_btns = row![].spacing(8);
    if app.drop.as_ref().is_some_and(|d| !d.receive_is_default) {
        drop_btns = drop_btns.push(btn(p, Btn::Ghost, "Use Documents", Some(Message::ResetDrop)));
    }
    drop_btns = drop_btns.push(btn(p, Btn::Plain, "Change folder…", Some(Message::ChangeDrop)));

    column![
        panel_head(
            p,
            "Step 2 · Handshake",
            "Read this code to the old PC",
            "On the old computer, WINCI is asking for a 6-digit code. Type the numbers below over there to \
             confirm the two machines are paired.",
        ),
        codebox,
        card(
            p,
            column![
                eyebrow(p, "No cable? Pair by IP address"),
                body(
                    format!(
                        "If the old PC can’t find this one automatically, choose “Enter IP” over there and \
                         type one of these addresses. Port is {port}."
                    ),
                    p.ink_2
                ),
                chips,
            ]
            .spacing(10)
        ),
        card(
            p,
            column![
                row![eyebrow(p, "Where incoming data lands"), space().width(Fill), drop_btns]
                    .align_y(Alignment::Center),
                row![body("Received files go to ", p.ink_2), mono(landing, style::S_0, p.ink), body(free, p.ink_2)]
                    .wrap(),
                text(
                    "If this drive is short on space, pick a bigger one before the old PC connects. WINCI \
                     remembers the choice."
                )
                .size(style::S_SM)
                .style(style::color(p.slate)),
            ]
            .spacing(10)
        ),
        waiting(app, true, p.slate, body("Listening for the old PC to connect…", p.ink).into()),
    ]
    .spacing(18)
    .into()
}

/* ---------- select (sender) ---------- */

fn source_meta<'a>(p: Pal, s: &SourceGroup) -> Element<'a, Message> {
    let items = if s.kind == "file" { "file".to_string() } else { format!("{} items", count(s.items)) };
    column![
        text(bytes(s.bytes as f64)).font(MONO).size(style::S_SM).style(style::color(p.ink)),
        mono(items, style::S_SM, p.ink_2),
    ]
    .align_x(Alignment::End)
    .into()
}

fn source_text<'a>(p: Pal, s: &SourceGroup, browser_badge: bool) -> Element<'a, Message> {
    let mut title = row![text(s.label.clone()).font(style::SEMI)].spacing(8).align_y(Alignment::Center);
    if browser_badge && s.kind == "browser" {
        title = title.push(container(mono("BROWSER", 10.0, p.copper)).padding([1, 5]).style(style::badge(p.copper)));
    }
    column![title, mono(s.hint.clone(), style::S_SM, p.slate)].spacing(2).width(Fill).into()
}

fn select(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let custom = app.mode == TransferMode::Custom;

    let modes = row([
        (TransferMode::User, "User Transfer", "Your folders & browser data, found for you"),
        (TransferMode::Custom, "Custom Transfer", "Pick any files and folders in Explorer"),
    ]
    .into_iter()
    .map(|(m, title, sub)| {
        button(column![text(title).font(style::SEMI), text(sub).size(style::S_SM).style(style::color(p.ink_2))].spacing(3))
            .on_press(Message::SetMode(m))
            .style(style::tile(p, app.mode == m))
            .padding([12, 14])
            .width(Fill)
            .into()
    }))
    .spacing(10);

    let busy_note: Element<_> = if app.picking {
        mono("Measuring…", style::S_SM, p.slate).into()
    } else {
        space().into()
    };
    let pick_btn = |label: &'static str, dir: bool, into: PickInto| {
        btn(p, Btn::Plain, label, (!app.picking).then_some(Message::Pick(dir, into)))
    };

    let list: Element<_> = if custom {
        let rows: Element<_> = if app.picks.is_empty() {
            container(body(
                "Nothing picked yet. Add folders or files from anywhere on this PC — other drives included. \
                 They land in Documents on the new PC.",
                p.ink_2,
            ))
            .padding([22, 16])
            .center_x(Fill)
            .style(style::tinted(p.panel, p.hairline, style::RADIUS_SM))
            .into()
        } else {
            Column::with_children(app.picks.iter().map(|s| {
                container(
                    row![
                        text(if s.kind == "file" { "▤" } else { "▣" }).style(style::color(p.cobalt)),
                        source_text(p, s, false),
                        source_meta(p, s),
                        button(text("✕").size(style::S_SM))
                            .on_press(Message::RemovePick(s.id.clone()))
                            .style(style::btn(p, Btn::Ghost))
                            .padding([4, 8]),
                    ]
                    .spacing(14)
                    .align_y(Alignment::Center),
                )
                .padding([12, 16])
                .style(style::row_box(p, true))
                .into()
            }))
            .spacing(8)
            .into()
        };
        column![
            rows,
            row![
                pick_btn("+ Add folders", true, PickInto::Picks),
                pick_btn("+ Add files", false, PickInto::Picks),
                busy_note
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        ]
        .spacing(10)
        .into()
    } else {
        let rows = Column::with_children(app.sources.iter().map(|s| {
            let on = s.selected;
            let check = container(text(if on { "✓" } else { "" }).size(style::S_SM).style(style::color(iced::Color::WHITE)))
                .center(20)
                .style(style::tinted(
                    if on { p.cobalt } else { p.panel },
                    if on { p.cobalt } else { p.hairline },
                    5.0,
                ));
            let mut inner = column![row![check, source_text(p, s, true), source_meta(p, s)]
                .spacing(14)
                .align_y(Alignment::Center)];
            if let (true, Some(c)) = (on, &s.caveat) {
                inner = inner.push(
                    container(text(format!("⚠ {c}")).size(style::S_SM).style(style::color(p.ink_2)))
                        .padding([8, 10])
                        .width(Fill)
                        .style(style::tinted(p.copper_soft, p.copper_soft, style::RADIUS_SM)),
                );
            }
            button(inner.spacing(8))
                .on_press(Message::Toggle(s.id.clone()))
                .style(style::tile(p, on))
                .padding([12, 16])
                .width(Fill)
                .into()
        }))
        .spacing(8);
        column![
            rows,
            row![pick_btn("+ Add a folder", true, PickInto::Sources), busy_note]
                .spacing(8)
                .align_y(Alignment::Center)
        ]
        .spacing(10)
        .into()
    };

    // (groups chosen, bytes, items) for the current mode
    let chosen: Vec<&SourceGroup> = if custom {
        app.picks.iter().collect()
    } else {
        app.sources.iter().filter(|g| g.selected).collect()
    };
    let total_b: u64 = chosen.iter().map(|s| s.bytes).sum();
    let total_n: u64 = chosen.iter().map(|s| s.items).sum();
    let all_on = !app.sources.is_empty() && app.sources.iter().all(|g| g.selected);
    let left = if custom {
        btn(p, Btn::Ghost, "Clear list", (!app.picks.is_empty()).then_some(Message::ClearPicks))
    } else {
        btn(
            p,
            Btn::Ghost,
            if all_on { "Deselect all" } else { "Select all" },
            (!app.sources.is_empty()).then_some(Message::SelectAll(!all_on)),
        )
    };
    let totals = column![
        container(space().height(1)).width(Fill).style(style::rule(p)),
        row![
            left,
            space().width(Fill),
            text(bytes(total_b as f64)).font(MONO).size(style::S_1),
            mono(format!(" across {} items", count(total_n)), style::S_0, p.ink_2),
            space().width(18),
            btn(
                p,
                Btn::Primary,
                "Start transfer →",
                (!chosen.is_empty() && !app.picking && !app.testing).then_some(Message::StartTransfer),
            ),
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(16);

    let bench = row![
        btn(
            p,
            Btn::Plain,
            if app.testing { "Testing link…" } else { "Test link speed" },
            (!app.testing).then_some(Message::RunBench),
        ),
        mono(app.speed.clone(), style::S_SM, p.ink_2),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    column![
        panel_head(
            p,
            "Step 3 · Payload",
            "Choose what to carry over",
            if custom {
                "Pick exactly what to send. Everything in the list crosses the cable."
            } else {
                "WINCI found these on the old PC. Tick what should land on the new machine."
            },
        ),
        modes,
        list,
        totals,
        bench,
    ]
    .spacing(16)
    .into()
}

/* ---------- transfer ---------- */

fn transfer(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let g = &app.progress;
    let sending = app.role == Some(Role::Send);
    let running = g.state == "running" || g.state == "reconnecting";
    let pct = if g.bytes_total > 0 {
        (g.bytes_sent as f64 / g.bytes_total as f64 * 100.0).min(100.0) as f32
    } else {
        0.0
    };

    let meter = |k: &'static str, v: String, of: Option<String>| {
        let mut val = row![text(v).font(MONO).size(style::S_2).wrapping(text::Wrapping::None)]
            .align_y(Alignment::End)
            .spacing(6);
        if let Some(of) = of {
            val = val.push(mono(of, style::S_SM, p.slate).wrapping(text::Wrapping::None));
        }
        container(column![eyebrow(p, k), val].spacing(6)).padding([14, 16]).width(Fill).style(style::card(p))
    };
    let meters = row![
        meter("Throughput", rate(g.bytes_per_sec), None),
        meter("Moved", bytes(g.bytes_sent as f64), Some(format!("/ {}", bytes(g.bytes_total as f64)))),
        meter("Items", count(g.files_sent), Some(format!("/ {}", count(g.files_total)))),
        meter(
            "Time left",
            eta(g.bytes_total.saturating_sub(g.bytes_sent) as f64, g.bytes_per_sec),
            None
        ),
    ]
    .spacing(12);

    let bar = progress_bar(0.0..=100.0, pct).girth(10).style(move |_| progress_bar::Style {
        background: p.paper_2.into(),
        bar: p.cobalt.into(),
        border: iced::border::rounded(5).width(1).color(p.hairline),
    });

    let now_file = match (&g.current_file, g.state.as_str()) {
        (Some(f), _) => f.clone(),
        (None, "idle") => "Preparing…".into(),
        _ => " ".into(),
    };

    let mut col = column![
        panel_head(
            p,
            format!("Step {} · Crossing", if sending { 4 } else { 3 }),
            if sending { "Sending to the new PC" } else { "Receiving from the old PC" },
            "Keep the cable connected. Data moves directly between the two machines over the wire above — \
             nothing touches the internet.",
        ),
        meters,
        bar,
    ]
    .spacing(16);
    if g.state == "reconnecting" {
        col = col.push(warn_line(
            p,
            "The connection was interrupted or data arrived damaged. WINCI is reconnecting and will pick up \
             from the file that was in flight — nothing already received is sent again.",
        ));
    }
    col = col.push(mono(now_file, style::S_SM, p.ink_2));
    if g.state == "error" {
        col = col.push(warn_line(p, g.error.clone().unwrap_or_else(|| "Transfer stopped.".into())));
    }
    let action = if running {
        btn(
            p,
            Btn::Plain,
            if app.stopping { "Stopping…" } else { "Stop transfer" },
            (!app.stopping).then_some(Message::Stop),
        )
    } else {
        btn(p, Btn::Ghost, "← Start over", Some(Message::Reset))
    };
    col.push(actions(vec![action.into()])).into()
}

/* ---------- done ---------- */

fn badge_for(p: Pal, action: &str) -> (&'static str, iced::Color) {
    match action {
        "imported" => ("✓ IMPORTED", p.good),
        "skipped-not-fresh" => ("SKIPPED", p.copper),
        "skipped-not-installed" => ("NOT INSTALLED", p.copper),
        _ => ("FAILED", p.bad),
    }
}

fn import_row<'a>(app: &App, en: &ImportEntry) -> Element<'a, Message> {
    let p = app.pal();
    let busy = app.pending_browser.is_some();
    let mine = app.pending_browser.as_deref() == en.browser_label.as_deref() && busy;
    let action: Element<_> = match (en.action.as_str(), en.browser_label.clone()) {
        ("skipped-not-fresh", Some(bl)) => button(text(if mine { "Overwriting…" } else { "Overwrite?" }).size(12))
            .on_press_maybe((!busy).then_some(Message::Browser(bl, BrowserAction::Overwrite)))
            .style(style::btn(p, Btn::Danger))
            .padding([2, 8])
            .into(),
        ("skipped-not-installed", Some(bl)) => button(text(if mine { "Checking…" } else { "Open First" }).size(12))
            .on_press_maybe((!busy).then_some(Message::Browser(bl, BrowserAction::Retry)))
            .style(style::btn(p, Btn::Danger))
            .padding([2, 8])
            .into(),
        (action, _) => {
            let (label, c) = badge_for(p, action);
            container(mono(label, 10.0, c)).padding([1, 5]).style(style::badge(c)).into()
        }
    };
    let mut r = row![text(en.label.clone()).font(style::SEMI).width(160), action]
        .spacing(10)
        .align_y(Alignment::Center);
    if en.count > 0 {
        r = r.push(body(format!("{} files", count(en.count)), p.ink_2));
    }
    if let Some(d) = &en.detail {
        r = r.push(text(d.clone()).size(style::S_SM).style(style::color(p.ink_2)).width(Fill));
    }
    column![
        r,
        container(space().height(1)).width(Fill).style(style::rule(p))
    ]
    .spacing(6)
    .into()
}

fn import_card(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let inner: Element<_> = match &app.report {
        None => {
            let landed = app.received_dir.clone().unwrap_or_else(|| "Documents\\WINC Received".into());
            let mut c = column![
                row![body("Files landed in ", p.ink_2), mono(landed, style::S_0, p.ink), body(".", p.ink_2)].wrap(),
                body(
                    "Import moves them into your real folders — or under a folder you choose — and nothing there \
                     is overwritten; incoming duplicates are kept as “name (from old PC)”. Browser data imports \
                     only into browsers with no existing data; ones that already have data show an “Overwrite?” \
                     option, and ones not yet opened show “Open First”.",
                    p.ink_2
                ),
                segmented(
                    p,
                    vec![
                        ("Your user folders", !app.use_folder, Message::UseImportFolder(false)),
                        ("A folder you choose", app.use_folder, Message::UseImportFolder(true)),
                    ]
                ),
            ]
            .spacing(12);
            if app.use_folder {
                let mut r = row![mono(
                    app.import_folder.clone().unwrap_or_else(|| "No folder chosen yet".into()),
                    style::S_0,
                    p.ink_2
                )]
                .spacing(10)
                .align_y(Alignment::Center);
                if let Some(f) = app.folder_free {
                    r = r.push(mono(format!("{} free", bytes(f as f64)), style::S_SM, p.slate));
                }
                r = r.push(btn(p, Btn::Plain, "Choose folder…", Some(Message::ChooseImportFolder)));
                c = c.push(r);
                c = c.push(
                    text(
                        "Documents, Desktop, Pictures… land as subfolders of it; custom picks go straight in. \
                         Remembered for next time.",
                    )
                    .size(style::S_SM)
                    .style(style::color(p.slate)),
                );
            }
            let can = !app.importing
                && app.received_dir.is_some()
                && !(app.use_folder && app.import_folder.is_none());
            c = c.push(btn(
                p,
                Btn::Primary,
                if app.importing { "Importing…" } else { "Import into place" },
                can.then_some(Message::RunImport),
            ));
            if let Some(e) = &app.import_err {
                c = c.push(warn_line(p, format!("Import failed — {e}")));
            }
            c.into()
        }
        Some(r) => {
            let mut c = column![].spacing(6);
            if r.entries.is_empty() {
                c = c.push(body("Nothing to import.", p.ink_2));
            }
            for en in &r.entries {
                c = c.push(import_row(app, en));
            }
            c = c.push(
                text(
                    "A snapshot log of every file (source → destination) was saved as import-log-*.json in the \
                     WINC Received folder.",
                )
                .size(12)
                .style(style::color(p.slate)),
            );
            if let Some(e) = &app.import_err {
                c = c.push(warn_line(p, e.clone()));
            }
            c.into()
        }
    };
    card(p, column![eyebrow(p, "Import into place"), inner].spacing(10)).into()
}

fn done(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let sending = app.role == Some(Role::Send);
    let g = &app.progress;
    let seal = container(text("✓").size(30).font(style::BOLD).style(style::color(iced::Color::WHITE)))
        .center(64)
        .style(style::tinted(p.good, p.good, 32.0));
    let hero = column![
        seal,
        text(if sending { "Everything sent" } else { "Everything landed" })
            .font(style::SEMI)
            .size(style::S_3),
        body(
            format!(
                "{} across {} items crossed the cable.",
                bytes(g.bytes_total as f64),
                count(g.files_total)
            ),
            p.ink_2
        ),
    ]
    .spacing(10)
    .align_x(Alignment::Center);

    let mut col = column![
        container(hero).center_x(Fill).padding([10, 0]),
        card(
            p,
            column![
                eyebrow(p, "What next"),
                bullets(
                    p,
                    &[
                        "Open the new PC and check a few folders landed where you expect.",
                        "Sign into your browser to finish syncing bookmarks and history.",
                        "Saved passwords only unlock under the same Windows / Microsoft account.",
                        "You can now safely unplug the cable.",
                    ]
                ),
            ]
            .spacing(10)
        ),
    ]
    .spacing(16);
    if !sending {
        col = col.push(import_card(app));
    }
    col.push(actions(vec![btn(p, Btn::Plain, "Transfer something else", Some(Message::Reset)).into()]))
        .into()
}
