//! In-app activity log: a bottom drawer. Ctrl+L toggles, Esc closes.
//! Reads the backend's ring buffer directly (same process now).

use super::format::clock;
use super::style::{self, Btn, MONO};
use super::widgets::{eyebrow, mono};
use super::{App, Message, LOG_SCROLL};
use crate::logbus::LogEntry;
use iced::widget::{button, column, container, pick_list, row, scrollable, space, text, text_input, Column};
use iced::{Alignment, Element, Fill, Padding};

/// Laying out every one of 3000 rows each frame is what makes a log view
/// janky; the tail is what anyone actually reads.
const MAX_ROWS: usize = 400;

pub const LEVELS: [(&str, &str); 5] = [
    ("error", "Errors"),
    ("warn", "Warnings"),
    ("info", "Info"),
    ("debug", "Debug"),
    ("trace", "Trace (every file)"),
];

/// Lower = more severe.
fn rank(level: &str) -> u8 {
    match level {
        "error" => 0,
        "warn" => 1,
        "info" => 2,
        "debug" => 3,
        _ => 4,
    }
}

/// Rows passing the level floor and the text filter, newest last.
pub fn shown(app: &App) -> Vec<&LogEntry> {
    let q = app.log_filter.trim().to_lowercase();
    let floor = rank(app.log_floor);
    let hits: Vec<&LogEntry> = app
        .log_rows
        .iter()
        .filter(|e| {
            rank(&e.level) <= floor
                && (q.is_empty() || e.msg.to_lowercase().contains(&q) || e.target.to_lowercase().contains(&q))
        })
        .collect();
    let skip = hits.len().saturating_sub(MAX_ROWS);
    hits.into_iter().skip(skip).collect()
}

#[derive(Clone, PartialEq)]
struct Level(&'static str, &'static str);

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1)
    }
}

pub fn drawer(app: &App) -> Element<'_, Message> {
    let p = app.pal();
    let rows = shown(app);
    let errors = app.log_rows.iter().filter(|e| e.level == "error").count();

    let levels: Vec<Level> = LEVELS.iter().map(|(v, l)| Level(v, l)).collect();
    let current = levels.iter().find(|l| l.0 == app.log_floor).cloned();
    let small = |label: &'static str, msg: Message| {
        button(text(label).size(style::S_SM))
            .on_press(msg)
            .style(style::btn(p, Btn::Ghost))
            .padding([4, 10])
    };

    let mut count = row![mono(format!("{} shown", rows.len()), style::S_SM, p.slate)];
    if errors > 0 {
        count = count.push(mono(
            format!(" · {errors} error{}", if errors > 1 { "s" } else { "" }),
            style::S_SM,
            p.bad,
        ));
    }

    let bar = row![
        pick_list(levels, current, |l: Level| Message::LogLevel(l.0))
            .text_size(style::S_SM)
            .padding([4, 8]),
        text_input("Filter…", &app.log_filter)
            .on_input(Message::LogFilter)
            .size(style::S_SM)
            .padding([5, 8])
            .width(200)
            .style(style::input(p)),
        count,
        space().width(Fill),
        small(if app.log_copied { "Copied" } else { "Copy" }, Message::LogCopy),
        small("Clear", Message::LogClear),
        small("Log file", Message::LogOpenFile),
        small(if app.log_tall { "Shrink" } else { "Expand" }, Message::LogTall),
        small("Close", Message::ToggleLogs),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let body: Element<_> = if rows.is_empty() {
        container(mono(
            if app.log_filter.is_empty() {
                "Nothing at this level yet."
            } else {
                "Nothing at this level yet matching that filter."
            },
            style::S_SM,
            p.slate,
        ))
        .padding(12)
        .into()
    } else {
        Column::with_children(rows.into_iter().map(|e| {
            let c = match e.level.as_str() {
                "error" => p.bad,
                "warn" => p.warn,
                "debug" | "trace" => p.slate,
                _ => p.ink,
            };
            row![
                mono(clock(e.ts), 12.0, p.slate).width(96),
                mono(e.level.to_uppercase(), 12.0, c).width(48),
                mono(e.target.clone(), 12.0, p.slate).width(150),
                text(e.msg.clone()).font(MONO).size(12).style(style::color(c)).width(Fill),
            ]
            .spacing(10)
            .into()
        }))
        .spacing(2)
        .padding(Padding::new(8.0).right(16))
        .into()
    };

    let head = row![eyebrow(p, "Activity log"), space().width(16), bar]
        .align_y(Alignment::Center)
        .padding([8, 16]);

    container(
        column![
            container(space().height(1)).width(Fill).style(style::rule(p)),
            head,
            // anchored to the bottom: stays on the newest line until scrolled up
            scrollable(body).id(LOG_SCROLL).anchor_bottom().height(Fill).width(Fill),
        ]
        .height(Fill),
    )
    .height(if app.log_tall { 520 } else { 280 })
    .width(Fill)
    .style(style::fill(p.panel))
    .into()
}
