//! Window frame: top bar, role picker, the cable, the step rail, the log drawer.

use super::style::{self, Btn, Pal, MONO};
use super::widgets::{body, eyebrow, mono, Cable};
use super::{logview, steps, App, Message, Role, Step, MAIN_SCROLL};
use iced::widget::{button, canvas, column, container, image, row, scrollable, space, stack, text};
use iced::{Alignment, Element, Fill, Length, Padding};

pub fn view(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let main: Element<_> = match app.role {
        None => role_picker(p),
        Some(_) => column![
            cable(app),
            row![
                rail(app),
                container(
                    scrollable(container(steps::view(app)).padding(Padding::ZERO.right(14)))
                        .id(MAIN_SCROLL)
                        .height(Fill)
                )
                    .width(Fill)
                    .height(Fill),
            ]
            .spacing(22)
            .padding(Padding::new(22.0).top(4))
            .height(Fill),
        ]
        .height(Fill)
        .into(),
    };

    let mut page = column![topbar(app), main].height(Fill);
    if app.logs_open {
        page = page.push(logview::drawer(app));
    }
    container(page)
        .width(Fill)
        .height(Fill)
        .style(style::fill(p.paper))
        .into()
}

fn topbar(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let brand = row![
        text("WIN").font(style::BOLD).size(style::S_1),
        text("CI").font(style::BOLD).size(style::S_1).style(style::color(p.copper)),
        space().width(10),
        mono("CABLE OR IP DATA CROSSING", style::S_SM, p.slate),
    ]
    .align_y(Alignment::Center);

    let errors = app.unseen_errors();
    let mut logs = row![text("Logs")].spacing(8).align_y(Alignment::Center);
    if !app.logs_open && errors > 0 {
        logs = logs.push(
            container(text(errors.to_string()).size(11).style(style::color(iced::Color::WHITE)))
                .padding([1, 6])
                .style(style::tinted(p.bad, p.bad, 9.0)),
        );
    }
    let mut actions = row![button(logs)
        .on_press(Message::ToggleLogs)
        .style(style::btn(p, Btn::Ghost))
        .padding([6, 12])]
    .spacing(4);
    if app.role.is_some() {
        actions = actions.push(
            button("Start over")
                .on_press(Message::Reset)
                .style(style::btn(p, Btn::Ghost))
                .padding([6, 12]),
        );
    }
    column![
        row![brand, space().width(Fill), actions]
            .align_y(Alignment::Center)
            .padding([10, 22]),
        container(space().height(1)).width(Fill).style(style::rule(p)),
    ]
    .into()
}

fn role_picker(p: Pal) -> Element<'static, Message> {
    let card = |glyph: &'static str, title: &'static str, blurb: &'static str, cta: Element<'static, Message>| {
        container(
            column![
                eyebrow(p, glyph),
                text(title).font(style::SEMI).size(style::S_2),
                body(blurb, p.ink_2),
                space().height(6),
                cta,
            ]
            .spacing(8),
        )
        .padding(24)
        .width(Fill)
        .style(style::card(p))
    };
    let send = card(
        "On the OLD PC",
        "Send my data",
        "This computer holds the files. Pick what to carry over and stream it to the new machine.",
        button(text("Start sending →"))
            .on_press(Message::SetRole(Role::Send))
            .style(style::btn(p, Btn::Primary))
            .padding([10, 18])
            .into(),
    );
    let recv = card(
        "On the NEW PC",
        "Receive data",
        "This computer is the destination. Show a pairing code to the old PC and stand by.",
        button(text("Wait for a sender"))
            .on_press(Message::SetRole(Role::Receive))
            .style(style::btn(p, Btn::Plain))
            .padding([10, 18])
            .into(),
    );
    let logo = image(image::Handle::from_bytes(
        include_bytes!("../../icons/128x128@2x.png").as_slice(),
    ))
    .width(84)
    .height(84);

    let inner = column![
        logo,
        eyebrow(p, "WINCI · PC-to-PC data crossing"),
        text("Move a whole PC\nacross one wire.")
            .font(style::BOLD)
            .size(style::S_4)
            .align_x(Alignment::Center),
        container(
            body(
                "Connect the two computers with a Thunderbolt / USB4 cable — or use any Wi-Fi or Ethernet \
                 network by entering the new PC's IP address. WINCI carries your files and browser data \
                 straight across — no cloud, no accounts, end-to-end encrypted.",
                p.ink_2,
            )
            .align_x(Alignment::Center)
        )
        .max_width(560),
        space().height(16),
        row![send, recv].spacing(18),
    ]
    .spacing(10)
    .align_x(Alignment::Center)
    .max_width(760);

    scrollable(container(inner).center_x(Fill).padding([40, 22])).height(Fill).into()
}

fn cable(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let flowing = app.flowing();
    let reconnecting = app.progress.state == "reconnecting";
    // paired counts as linked over Wi-Fi / Ethernet too, not just the cable
    let linked = app.peer.is_some() || flowing;

    let node = |role: &'static str, name: String, ip: String, active: bool| {
        container(
            column![
                mono(role.to_uppercase(), style::S_SM, p.slate),
                text(name).font(style::SEMI).size(style::S_1),
                mono(ip, style::S_SM, p.ink_2),
            ]
            .spacing(2),
        )
        .padding([12, 16])
        .width(190)
        .style(style::node(p, active))
    };
    let (this_role, peer_role) = match app.role {
        Some(Role::Receive) => ("New device", "Old device"),
        _ => ("Old device", "New device"),
    };
    let me = node(
        this_role,
        "This PC".into(),
        app.link.local_ip.clone().unwrap_or_else(|| "—".into()),
        app.role.is_some(),
    );
    let them = node(
        peer_role,
        app.peer.as_ref().map_or("Waiting…".into(), |p| p.name.clone()),
        app.peer.as_ref().map_or("—".into(), |p| p.ip.clone()),
        app.peer.is_some(),
    );

    let (dot, label) = if reconnecting {
        (p.warn, "Reconnecting…".to_string())
    } else if flowing {
        (p.cobalt, super::format::rate(app.progress.bytes_per_sec))
    } else if linked {
        (p.copper, if app.link.up { "Linked".into() } else { "Linked · network".to_string() })
    } else if app.link.up {
        (p.slate, "Cable up · finding peer".into())
    } else {
        (p.slate, "No link".into())
    };
    let readout = container(
        row![
            container(space().width(7).height(7)).style(style::tinted(dot, dot, 4.0)),
            mono(label, style::S_SM, p.ink),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([6, 14])
    .style(style::tinted(p.panel, p.hairline, 100.0));

    let wire = stack![
        canvas(Cable {
            pal: p,
            linked,
            flowing,
            phase: app.phase(),
        })
        .width(Fill)
        .height(96),
        container(readout).center(Fill),
    ]
    .width(Fill)
    .height(96);

    row![me, wire, them]
        .align_y(Alignment::Center)
        .padding(Padding::new(22.0).top(22).bottom(18))
        .into()
}

const SEND_STEPS: &[(Step, &str, &str)] = &[
    (Step::Connect, "Connect cable", "Thunderbolt / USB4"),
    (Step::Pair, "Pair devices", "Enter 6-digit code"),
    (Step::Select, "Choose data", "Files & browser"),
    (Step::Transfer, "Transfer", "Stream to new PC"),
    (Step::Done, "Done", "Verify & finish"),
];

const RECEIVE_STEPS: &[(Step, &str, &str)] = &[
    (Step::Connect, "Connect cable", "Thunderbolt / USB4"),
    (Step::Code, "Show code", "Read it to old PC"),
    (Step::Transfer, "Receive", "Incoming data"),
    (Step::Done, "Done", "Everything landed"),
];

fn rail(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let steps = match app.role {
        Some(Role::Receive) => RECEIVE_STEPS,
        _ => SEND_STEPS,
    };
    let active = steps.iter().position(|(id, ..)| *id == app.step);

    let mut col = column![].spacing(4).width(250);
    for (i, (_, label, sub)) in steps.iter().enumerate() {
        let (done, on) = match active {
            Some(a) => (i < a, i == a),
            None => (false, false),
        };
        let (num_bg, num_line, num_fg, num) = if done {
            (p.good, p.good, iced::Color::WHITE, "✓".to_string())
        } else if on {
            (p.panel, p.cobalt, p.cobalt, (i + 1).to_string())
        } else {
            (p.panel, p.hairline, p.slate, (i + 1).to_string())
        };
        let badge = container(text(num).font(MONO).size(style::S_SM).style(style::color(num_fg)))
            .center(26)
            .style(style::tinted(num_bg, num_line, 13.0));
        let fg = if on {
            p.ink
        } else if done {
            p.ink_2
        } else {
            p.slate
        };
        let item = row![
            badge,
            column![
                text(*label).font(style::SEMI).style(style::color(fg)),
                text(*sub).size(style::S_SM).style(style::color(p.slate)),
            ]
            .spacing(1),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        col = col.push(
            container(item)
                .padding([10, 8])
                .width(Fill)
                .style(style::tinted(
                    if on { p.paper_2 } else { iced::Color::TRANSPARENT },
                    iced::Color::TRANSPARENT,
                    style::RADIUS_SM,
                )),
        );
    }

    let foot = column![
        container(space().height(1)).width(Fill).style(style::rule(p)),
        mono(
            format!(
                "LINK · {}",
                app.link.adapter.clone().unwrap_or_else(|| "none".into())
            ),
            style::S_SM,
            p.slate
        ),
        mono(
            app.link.local_ip.clone().unwrap_or_else(|| "no address".into()),
            style::S_SM,
            p.slate
        ),
    ]
    .spacing(6);

    row![
        column![col, space().height(Fill), foot].height(Fill).width(250),
        container(space().width(1)).height(Fill).style(style::rule(p)),
    ]
    .spacing(18)
    .height(Length::Fill)
    .into()
}
