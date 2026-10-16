//! The terminal frame: top bar, step rail, the scrolling step body, the log
//! panel, the key-hint footer, and the modals (file browser, quit check).
//! Also where each frame's hits are collected and focus is settled.

use super::app::{log_line, Msg, Pk, TuiApp, IN_FILTER, IN_PATH, PK_LIST};
use super::kit::{
    self, accent, bad, bold, btn, copper, dim, focus_style, good, ink, soft, width, BtnKind, Hit, Kind, Region, Seg, Ui,
};
use super::screens;
use crate::ui::{Role, Step};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::widgets::{Clear, Widget};
use ratatui::Frame;

/// Widest the step body gets; wider lines are hard to read.
const BODY_MAX: u16 = 104;

const SEND_STEPS: &[(Step, &str)] = &[
    (Step::Connect, "Connect"),
    (Step::Pair, "Pair"),
    (Step::Select, "Choose data"),
    (Step::Transfer, "Transfer"),
    (Step::Done, "Done"),
];
const RECEIVE_STEPS: &[(Step, &str)] = &[
    (Step::Connect, "Connect"),
    (Step::Code, "Show code"),
    (Step::Transfer, "Receive"),
    (Step::Done, "Done"),
];

pub fn draw(f: &mut Frame, app: &mut TuiApp) {
    // a frame that moved focus (a new step's default) is drawn again, so the
    // focus mark shows straight away
    if draw_once(f, app) {
        f.buffer_mut().reset();
        draw_once(f, app);
    }
}

/// One pass. Returns true when focus had to move to something on this frame.
fn draw_once(f: &mut Frame, app: &mut TuiApp) -> bool {
    let area = f.area();
    let buf = f.buffer_mut();
    let focus = app.focus.clone();
    let modal = app.picker.is_some() || app.confirm_quit;

    // rows: top bar · rail · body · [log panel] · footer
    let mut y = area.y;
    let top = Rect::new(area.x, y, area.width, 1.min(area.height));
    y += top.height;
    let rail = if app.role.is_some() && area.height > 6 {
        let r = Rect::new(area.x, y, area.width, 2);
        y += 2;
        Some(r)
    } else {
        y = (y + 1).min(area.bottom());
        None
    };
    let used = y - area.y;
    let footer_h = 1.min(area.height.saturating_sub(used));
    let left = area.height.saturating_sub(used + footer_h);
    let log_h = if app.logs_open {
        let want = if app.log_tall { area.height * 7 / 10 } else { area.height * 2 / 5 };
        want.max(8).min(left.saturating_sub(3))
    } else {
        0
    };
    let body = Rect::new(area.x, y, area.width, left.saturating_sub(log_h));
    let log = Rect::new(area.x, body.bottom(), area.width, log_h);
    let footer = Rect::new(area.x, log.bottom(), area.width, footer_h);
    app.body_area = body;
    app.log_area = log;

    let mut top_hits = topbar(app, buf, top, &focus);
    if let Some(r) = rail {
        draw_rail(app, buf, r);
    }

    // the step body, laid out on a tall canvas and scrolled into the body rect
    let cw = body.width.saturating_sub(4).min(BODY_MAX);
    let mut ui = Ui::new(cw, kit::CANVAS_ROWS, &focus, app.tick);
    screens::body(app, &mut ui);
    let content_h = ui.y;
    let autofocus = ui.autofocus.take();

    // keep the focused item in view after keyboard moves
    if app.reveal && !modal {
        if let Some(h) = ui.hits.iter().find(|h| h.key == app.focus) {
            let (top_y, bot) = (h.virt.y, h.virt.bottom());
            if top_y < app.body_scroll {
                app.body_scroll = top_y.saturating_sub(1);
            } else if bot > app.body_scroll + body.height {
                app.body_scroll = (bot + 1).saturating_sub(body.height);
            }
        }
        app.reveal = false;
    }
    app.body_scroll = app.body_scroll.min(content_h.saturating_sub(body.height));
    let body_inner = Rect::new(body.x + 2, body.y, cw, body.height);
    let mut body_hits = blit(&ui, buf, body_inner, app.body_scroll, Region::Body);
    if content_h > body.height && body.width > cw + 3 {
        scrollbar(buf, Rect::new(body.right() - 1, body.y, 1, body.height), app.body_scroll, content_h);
    }
    if app.body_scroll + body.height < content_h && body.height > 2 && body.width > 30 {
        let more = kit::a(" ↓ more below · PgDn ");
        let mw = width(&more) as u16;
        buf.set_string(body.right().saturating_sub(mw + 2), body.bottom() - 1, more, dim().patch(focus_style()));
    }

    let mut log_hits = if log_h > 0 { log_panel(app, buf, log, &focus) } else { Vec::new() };
    draw_footer(app, buf, footer);

    let hits = if app.picker.is_some() {
        picker(app, buf, area, &focus)
    } else if app.confirm_quit {
        confirm_quit(buf, area, &focus)
    } else {
        body_hits.append(&mut log_hits);
        body_hits.append(&mut top_hits);
        body_hits
    };

    // nothing valid focused (a new step, a closed modal): the step's default,
    // else the first thing on screen
    let mut moved = false;
    if !hits.iter().any(|h| h.key == app.focus) {
        // only something on screen: Enter on a button out of sight surprises
        let first_visible = hits.iter().find(|h| h.screen.is_some()).or(hits.first());
        let pick = autofocus
            .filter(|k| !modal && hits.iter().any(|h| h.key == *k && h.screen.is_some()))
            .or_else(|| first_visible.map(|h| h.key.clone()))
            .unwrap_or_default();
        moved = pick != app.focus && !pick.is_empty();
        app.focus = pick;
    }
    app.hits = hits;
    moved
}

/// Copy canvas rows [scroll, scroll + area.height) into `area`, and place the
/// canvas's hits on screen.
pub fn blit(ui: &Ui, dst: &mut Buffer, area: Rect, scroll: u16, region: Region) -> Vec<Hit> {
    let src = &ui.buf;
    let w = area.width.min(src.area.width);
    for row in 0..area.height {
        let sy = scroll + row;
        if sy >= src.area.height {
            break;
        }
        for col in 0..w {
            let (dx, dy) = (area.x + col, area.y + row);
            if dx < dst.area.right() && dy < dst.area.bottom() {
                dst[(dx, dy)] = src[(col, sy)].clone();
            }
        }
    }
    ui.hits
        .iter()
        .map(|h| {
            let mut h = h.clone();
            h.region = region;
            let r = h.virt;
            h.screen = (r.bottom() > scroll && r.y < scroll + area.height && r.x < w).then(|| {
                let y0 = r.y.max(scroll);
                let y1 = r.bottom().min(scroll + area.height);
                Rect::new(area.x + r.x, area.y + y0 - scroll, r.width.min(w - r.x), y1 - y0)
            });
            h
        })
        .collect()
}

fn scrollbar(buf: &mut Buffer, r: Rect, scroll: u16, content: u16) {
    if r.height == 0 || content == 0 {
        return;
    }
    let thumb = ((r.height as u32 * r.height as u32) / content as u32).max(1) as u16;
    let max = content.saturating_sub(r.height).max(1);
    let pos = ((r.height - thumb) as u32 * scroll as u32 / max as u32) as u16;
    for i in 0..r.height {
        let on = i >= pos && i < pos + thumb;
        let s = if on { "┃" } else { "│" };
        buf.set_string(r.x, r.y + i, kit::a(s), if on { soft() } else { dim() });
    }
}

/// Draw a small canvas straight onto the screen (no scrolling) and return its hits.
fn overlay(ui: &Ui, buf: &mut Buffer, area: Rect, region: Region) -> Vec<Hit> {
    blit(ui, buf, area, 0, region)
}

/* ---------- top bar + rail + footer ---------- */

fn topbar(app: &TuiApp, buf: &mut Buffer, r: Rect, focus: &str) -> Vec<Hit> {
    if r.height == 0 {
        return Vec::new();
    }
    let mut ui = Ui::new(r.width, 1, focus, app.tick);
    // right: Logs · Start over · Quit
    let errors = app.unseen_errors();
    let (logs, kind) = if !app.logs_open && errors > 0 {
        (format!("Logs ({errors}!)"), BtnKind::Danger)
    } else {
        ("Logs".to_string(), BtnKind::Ghost)
    };
    let mut btns = vec![btn("top:logs", kind, logs, Some(Msg::ToggleLogs))];
    if app.role.is_some() {
        btns.push(btn("top:reset", BtnKind::Ghost, "Start over", Some(Msg::Reset)));
    }
    btns.push(btn("top:quit", BtnKind::Ghost, "Quit", Some(Msg::Quit)));
    let total: u16 = btns.iter().map(|b| width(&kit::a(&b.label)) as u16 + 5).sum();
    let mut x = r.width.saturating_sub(total);
    let right_start = x;
    for b in &btns {
        x += ui.button_at(x, 0, b) + 1;
    }

    // left: brand, version, role, link
    ui.w = right_start.saturating_sub(1).max(1);
    let mut c = ui.put(1, 0, "WIN", bold());
    c += ui.put(1 + c, 0, "CI", copper().patch(bold()));
    c += ui.put(1 + c, 0, &format!(" v{}", env!("CARGO_PKG_VERSION")), dim());
    let role = match app.role {
        Some(Role::Send) => "  · Sending",
        Some(Role::Receive) => "  · Receiving",
        None => "",
    };
    c += ui.put(1 + c, 0, role, soft());
    if app.role.is_some() {
        let (dot, st, text) = if app.link.up {
            (
                "●",
                copper(),
                format!(
                    " Linked · cable {}",
                    app.link.local_ip.clone().unwrap_or_default()
                ),
            )
        } else if app.peer.is_some() && matches!(app.step, Step::Select | Step::Transfer | Step::Done) {
            ("●", good(), format!(" Linked · network {}", app.peer.as_ref().map_or("", |p| p.ip.as_str())))
        } else {
            ("○", dim(), " no cable link".to_string())
        };
        c += ui.put(1 + c, 0, "   ", ink());
        c += ui.put(1 + c, 0, dot, st);
        ui.put(1 + c, 0, &text, soft());
    }
    ui.w = r.width;
    overlay(&ui, buf, r, Region::Top)
}

fn draw_rail(app: &TuiApp, buf: &mut Buffer, r: Rect) {
    let steps = match app.role {
        Some(Role::Receive) => RECEIVE_STEPS,
        _ => SEND_STEPS,
    };
    let active = steps.iter().position(|(s, _)| *s == app.step).unwrap_or(0);
    let mut ui = Ui::new(r.width, 2, "", app.tick);
    let mut x = 2;
    for (i, (_, label)) in steps.iter().enumerate() {
        if i > 0 {
            x += ui.put(x, 0, " ── ", dim());
        }
        let (num, st) = if i < active {
            ("✓".to_string(), good())
        } else if i == active {
            ((i + 1).to_string(), accent().patch(bold()))
        } else {
            ((i + 1).to_string(), dim())
        };
        x += ui.put(x, 0, &num, st);
        x += ui.put(
            x,
            0,
            &format!(" {label}"),
            if i == active { bold() } else if i < active { soft() } else { dim() },
        );
    }
    ui.put(0, 1, &"─".repeat(r.width as usize), dim());
    overlay(&ui, buf, r, Region::Top);
}

fn draw_footer(app: &TuiApp, buf: &mut Buffer, r: Rect) {
    if r.height == 0 {
        return;
    }
    let hint = if app.picker.is_some() {
        "↑↓ move · Enter/→ open · ←/Bksp up · Space mark · / type a path (Tab completes) · . hidden · Esc cancel"
    } else if app.confirm_quit {
        "y quit · n / Esc keep going"
    } else if app.hits.iter().any(|h| h.key == app.focus && h.kind == Kind::Input) {
        "type · Enter submit · Tab / ↑↓ next field · Ctrl+L logs · Ctrl+C quit"
    } else {
        "Tab/↑↓ move · Enter/Space select · or click · PgUp/PgDn/wheel scroll · Ctrl+L logs · q quit"
    };
    let right = "Shift+drag selects text";
    let mut ui = Ui::new(r.width, 1, "", app.tick);
    let rw = width(right) as u16;
    let room = r.width.saturating_sub(rw + 3);
    ui.w = room.max(1);
    ui.put(1, 0, hint, dim());
    ui.w = r.width;
    if r.width > rw + 40 {
        ui.put(r.width - rw - 1, 0, right, dim().add_modifier(Modifier::ITALIC));
    }
    overlay(&ui, buf, r, Region::Top);
}

/* ---------- log panel ---------- */

fn log_panel(app: &TuiApp, buf: &mut Buffer, r: Rect, focus: &str) -> Vec<Hit> {
    if r.height < 3 {
        return Vec::new();
    }
    let rows = crate::ui::logview::filter(&app.log_rows, app.log_floor, &app.filter_edit.text);
    let errors = app.log_rows.iter().filter(|e| e.level == "error").count();

    // header: title + buttons · level + filter · file
    let mut ui = Ui::new(r.width.saturating_sub(2), 4, focus, app.tick);
    ui.x = 0;
    ui.put(0, 0, &"─".repeat(r.width as usize), dim());
    ui.y = 1;
    let mut c = ui.put(0, 1, "ACTIVITY LOG  ", bold());
    c += ui.put(c, 1, &format!("{} shown", rows.len()), dim());
    if errors > 0 {
        c += ui.put(c, 1, &format!(" · {errors} error{}", if errors > 1 { "s" } else { "" }), bad());
    }
    let btns = [
        btn("log:copy", BtnKind::Ghost, if app.log_copied { "Copied" } else { "Copy" }, Some(Msg::LogCopy)),
        btn("log:clear", BtnKind::Ghost, "Clear", Some(Msg::LogClear)),
        btn("log:tall", BtnKind::Ghost, if app.log_tall { "Shrink" } else { "Expand" }, Some(Msg::LogTall)),
        btn("log:close", BtnKind::Ghost, "Close", Some(Msg::ToggleLogs)),
    ];
    let total: u16 = btns.iter().map(|b| width(&kit::a(&b.label)) as u16 + 5).sum();
    let mut x = ui.w.saturating_sub(total).max(c + 2);
    for b in &btns {
        x += ui.button_at(x, 1, b) + 1;
    }
    ui.y = 2;
    let levels: [(&'static str, &'static str); 5] = [
        ("error", "Errors"),
        ("warn", "Warnings"),
        ("info", "Info"),
        ("debug", "Debug"),
        ("trace", "Trace"),
    ];
    let segs = levels
        .iter()
        .map(|(v, l)| Seg {
            key: match *v {
                "error" => "log:lv:error",
                "warn" => "log:lv:warn",
                "info" => "log:lv:info",
                "debug" => "log:lv:debug",
                _ => "log:lv:trace",
            },
            label: l,
            on: app.log_floor == *v,
            msg: Msg::LogLevel(v),
        })
        .collect();
    // level picker, then the filter beside it when there's room
    ui.seg(segs);
    let seg_w: u16 = levels.iter().map(|(_, l)| width(l) as u16 + 5).sum();
    if ui.w > seg_w + 24 && ui.y == 3 {
        let (ox, ow) = (ui.x, ui.w);
        ui.x = seg_w + 1;
        ui.w = ow.saturating_sub(seg_w + 1).min(40);
        ui.y = 2;
        ui.input(IN_FILTER, &app.filter_edit, "Filter…", Msg::Noop, 0);
        ui.x = ox;
        ui.w = ow;
    } else {
        ui.y = 3;
    }
    let file = format!("file: {}", app.log_path);
    let (x0, y0) = (0, ui.y.min(3));
    ui.put(x0, y0, &file, dim());
    let head = Rect::new(r.x + 1, r.y, r.width.saturating_sub(1), 4.min(r.height));
    let hits = overlay(&ui, buf, head, Region::Log);

    // the rows, newest at the bottom, wrapped
    let list = Rect::new(r.x + 1, r.y + 4, r.width.saturating_sub(2), r.height.saturating_sub(4));
    if list.height == 0 {
        return hits;
    }
    let mut lines: Vec<Vec<(String, ratatui::style::Style)>> = Vec::new();
    if rows.is_empty() {
        lines.push(vec![(
            if app.filter_edit.text.is_empty() {
                "Nothing at this level yet.".to_string()
            } else {
                "Nothing at this level yet matching that filter.".to_string()
            },
            dim(),
        )]);
    }
    for e in &rows {
        let st = match e.level.as_str() {
            "error" => bad(),
            "warn" => copper(),
            "debug" | "trace" => dim(),
            _ => ink(),
        };
        let text = log_line(e).replace("[winci::", "[");
        let ascii_text = kit::a(&text).into_owned();
        lines.extend(kit::wrap(&[(ascii_text, st)], list.width as usize));
    }
    let max_scroll = lines.len().saturating_sub(list.height as usize);
    let scroll = app.log_scroll.min(max_scroll);
    let end = lines.len() - scroll;
    let start = end.saturating_sub(list.height as usize);
    for (i, l) in lines[start..end].iter().enumerate() {
        let mut x = list.x;
        for (s, st) in l {
            let (nx, _) = buf.set_stringn(x, list.y + i as u16, s, list.right().saturating_sub(x) as usize, *st);
            x = nx;
        }
    }
    if scroll > 0 && list.width > 20 {
        let note = format!(" ↓ {scroll} newer line{} ", if scroll == 1 { "" } else { "s" });
        let note = kit::a(&note);
        let nw = width(&note) as u16;
        buf.set_string(list.right().saturating_sub(nw), list.bottom() - 1, note, accent().patch(focus_style()));
    }
    hits
}

/* ---------- modals ---------- */

fn modal_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2)).max(10.min(area.width));
    let h = h.min(area.height.saturating_sub(2)).max(5.min(area.height));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

/// Clear `r` and draw a border; returns the inside.
fn frame_box(buf: &mut Buffer, r: Rect, title: &str) -> Rect {
    Clear.render(r, buf);
    let mut ui = Ui::new(r.width, r.height, "", 0);
    ui.y = 0;
    ui.card(Some(title), |ui| {
        ui.y = r.height.saturating_sub(1);
    });
    overlay(&ui, buf, r, Region::Modal);
    Rect::new(r.x + 2, r.y + 1, r.width.saturating_sub(4), r.height.saturating_sub(2))
}

fn confirm_quit(buf: &mut Buffer, area: Rect, focus: &str) -> Vec<Hit> {
    let r = modal_rect(area, 60, 8);
    let inner = frame_box(buf, r, "Quit WINCI?");
    let mut ui = Ui::new(inner.width, inner.height, focus, 0);
    ui.text("Something is still in progress (a pairing, a transfer or an import). Quitting stops it.", soft());
    ui.gap();
    ui.buttons(vec![
        btn("q:no", BtnKind::Plain, "Keep going   n", Some(Msg::QuitAnswer(false))),
        btn("q:yes", BtnKind::Danger, "Stop and quit   y", Some(Msg::QuitAnswer(true))),
    ]);
    overlay(&ui, buf, inner, Region::Modal)
}

fn picker(app: &mut TuiApp, buf: &mut Buffer, area: Rect, focus: &str) -> Vec<Hit> {
    let Some(p) = app.picker.as_mut() else { return Vec::new() };
    let title = match p.purpose {
        super::picker::PickFor::Drop => "Where incoming data lands",
        super::picker::PickFor::ImportFolder => "Import into this folder",
        super::picker::PickFor::Into(_) => "Add files and folders",
    };
    let r = modal_rect(area, 100, area.height.saturating_sub(2));
    let inner = frame_box(buf, r, title);
    let mut ui = Ui::new(inner.width, inner.height, focus, 0);

    ui.input(IN_PATH, &p.path, "type a path…", Msg::Picker(Pk::PathSubmit), 0);
    if let Some(e) = &p.err {
        ui.warn(e);
    }
    let footer_rows = 3;
    let list_top = ui.y;
    let rows = inner.height.saturating_sub(list_top + footer_rows).max(1) as usize;
    p.page = rows;
    p.keep_in_view();
    let multi = p.multi();
    let list_focused = focus == PK_LIST;
    for (row, i) in (p.scroll..p.entries.len().min(p.scroll + rows)).enumerate() {
        let e = &p.entries[i];
        let y = list_top + row as u16;
        let cur = i == p.cursor;
        let x0 = 0;
        ui.put(x0, y, if cur { "▸" } else { " " }, accent());
        let mut x = 2;
        if multi {
            if !e.up {
                let marked = p.is_marked(&e.path);
                ui.put(x, y, if marked { "[✓]" } else { "[ ]" }, if marked { accent().patch(bold()) } else { dim() });
                ui.hit(PK_LIST, Rect::new(x, y, 3, 1), Msg::Picker(Pk::Mark(i)), Kind::List);
            }
            x += 4;
        }
        let name = if e.dir && !e.up { format!("{}/", e.name) } else { e.name.clone() };
        let mut st = if e.dir { accent() } else { ink() };
        if cur {
            st = if list_focused { st.patch(focus_style()) } else { st.patch(bold()) };
        }
        let size = if e.dir { String::new() } else { crate::ui::format::bytes(e.size as f64) };
        let sw = width(&size) as u16;
        let room = ui.w.saturating_sub(x + sw + 2);
        let saved = ui.w;
        ui.w = x + room;
        ui.put(x, y, &format!(" {name} "), st);
        ui.w = saved;
        ui.put(ui.w.saturating_sub(sw), y, &size, dim());
        ui.hit(PK_LIST, Rect::new(0, y, ui.w, 1), Msg::Picker(Pk::Click(i)), Kind::List);
    }
    if p.entries.is_empty() {
        ui.put(2, list_top, "(empty)", dim());
        ui.hit(PK_LIST, Rect::new(0, list_top, ui.w, 1), Msg::Noop, Kind::List);
    }

    // footer: what's marked, and the buttons
    ui.y = inner.height.saturating_sub(footer_rows);
    let note = if multi {
        format!(
            "{} marked · {} entries{}",
            p.marked.len(),
            p.entries.iter().filter(|e| !e.up).count(),
            if p.hidden { " · showing hidden" } else { "" }
        )
    } else {
        format!("in {}", p.dir.display())
    };
    let note = kit::fit_label(&note, ui.w as usize);
    ui.line(&[(&note, dim())]);
    let (confirm, label) = match p.result() {
        Some((_, label)) => (Some(Msg::Picker(Pk::Confirm)), label),
        None => (None, "Mark something to add".to_string()),
    };
    // long paths: keep the verb and the end, the part that differs
    let label = kit::fit_label(&label, (ui.w as usize).saturating_sub(64).max(20));
    ui.buttons(vec![
        btn("pk:ok", BtnKind::Primary, label, confirm),
        btn("pk:up", BtnKind::Ghost, "↑ Parent", Some(Msg::Picker(Pk::Parent))),
        btn("pk:home", BtnKind::Ghost, "~ Home", Some(Msg::Picker(Pk::Home))),
        btn(
            "pk:hidden",
            BtnKind::Ghost,
            if p.hidden { "Hide hidden" } else { "Show hidden" },
            Some(Msg::Picker(Pk::Hidden)),
        ),
        btn("pk:cancel", BtnKind::Ghost, "Cancel", Some(Msg::Picker(Pk::Cancel))),
    ]);
    // list first in Tab order, then the path field, then the buttons
    let mut hits = overlay(&ui, buf, inner, Region::Modal);
    let (list, rest): (Vec<Hit>, Vec<Hit>) = hits.drain(..).partition(|h| h.key == PK_LIST);
    let (path, buttons): (Vec<Hit>, Vec<Hit>) = rest.into_iter().partition(|h| h.key == IN_PATH);
    list.into_iter().chain(path).chain(buttons).collect()
}

#[cfg(test)]
mod tests {
    use super::super::app::tests::app;
    use super::*;
    use crate::model::{LinkStatus, SourceGroup};
    use crate::ui::{PairMode, PairStatus, TransferMode};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render(a: &mut TuiApp, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, a)).unwrap();
        // twice: the first frame settles focus
        t.draw(|f| draw(f, a)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn click_key(a: &mut TuiApp, key: &str) {
        let r = a
            .hits
            .iter()
            .find(|h| h.key == key)
            .and_then(|h| h.screen)
            .unwrap_or_else(|| panic!("{key} not on screen"));
        a.mouse(ratatui::crossterm::event::MouseEvent {
            kind: ratatui::crossterm::event::MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left),
            column: r.x,
            row: r.y,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        });
    }

    #[test]
    fn role_picker_renders_and_click_picks_a_role() {
        let (mut a, _rx) = app();
        let s = render(&mut a, 100, 40);
        assert!(s.contains("Move a whole PC across one wire."), "{s}");
        assert!(s.contains("Start sending"));
        assert!(s.contains("Wait for a sender"));
        assert_eq!(a.focus, "role:send");
        click_key(&mut a, "role:recv");
        assert_eq!(a.role, Some(Role::Receive));
        assert_eq!(a.step, Step::Connect);
    }

    #[test]
    fn connect_and_pair_screens() {
        let (mut a, _rx) = app();
        a.role = Some(Role::Send);
        a.link = LinkStatus {
            up: true,
            adapter: Some("thunderbolt0".into()),
            local_ip: Some("169.254.3.1".into()),
            kind: Some("thunderbolt".into()),
        };
        let s = render(&mut a, 100, 40);
        assert!(s.contains("Link up on thunderbolt0"), "{s}");
        assert!(s.contains("Linked · cable 169.254.3.1"));
        assert_eq!(a.focus, "next");

        a.step = Step::Pair;
        a.pair_mode = PairMode::Manual;
        a.pair_status = PairStatus::Found;
        a.addr_edit.set("192.168.1.24");
        a.code_edit.set("4186");
        a.focus.clear();
        let s = render(&mut a, 100, 40);
        assert!(s.contains("4 1 8 · 6 _ _"), "{s}");
        assert!(s.contains("192.168.1.24"));
        // Pair is disabled until all six digits are in
        assert!(!a.hits.iter().any(|h| h.key == "pair"));
        a.code_edit.set("418620");
        render(&mut a, 100, 40);
        assert!(a.hits.iter().any(|h| h.key == "pair"));
    }

    #[test]
    fn select_scrolls_focus_into_view_on_a_short_terminal() {
        let (mut a, _rx) = app();
        a.role = Some(Role::Send);
        a.step = Step::Select;
        a.mode = TransferMode::User;
        a.sources = (0..12)
            .map(|i| SourceGroup {
                id: format!("g{i}"),
                label: format!("Group {i}"),
                hint: format!("/data/g{i}"),
                kind: "folder".into(),
                path: Some(format!("/data/g{i}")),
                bytes: 1_000_000,
                items: 3,
                caveat: None,
                selected: i == 0,
            })
            .collect();
        let s = render(&mut a, 90, 24);
        assert!(s.contains("Group 0"), "{s}");
        assert!(!s.contains("Start transfer"));
        // walk focus down to Start transfer: the body follows
        for _ in 0..40 {
            if a.focus == "start" {
                break;
            }
            a.key(ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Tab,
                ratatui::crossterm::event::KeyModifiers::NONE,
            ));
            render(&mut a, 90, 24);
        }
        assert_eq!(a.focus, "start");
        let s = render(&mut a, 90, 24);
        assert!(s.contains("Start transfer"), "{s}");
        assert!(a.body_scroll > 0);
        // and clicking it works where it's drawn now
        let before = a.step;
        assert_eq!(before, Step::Select);
        assert!(a.hits.iter().find(|h| h.key == "start").unwrap().screen.is_some());
    }

    #[test]
    fn receiver_code_and_log_panel() {
        let (mut a, _rx) = app();
        a.role = Some(Role::Receive);
        a.step = Step::Code;
        a.code = "418 620".into();
        a.ips = vec!["169.254.3.1".into(), "192.168.1.9".into()];
        let s = render(&mut a, 100, 50);
        assert!(s.contains("418 620"), "{s}");
        assert!(s.contains("169.254.3.1 (direct link)"));
        crate::logbus::ui("error", "tui::test", "something broke");
        a.update(Msg::Backend(crate::backend::Event::Log));
        let s = render(&mut a, 100, 50);
        assert!(s.contains("Logs (1!)") || a.unseen_errors() > 0, "{s}");
        a.update(Msg::ToggleLogs);
        let s = render(&mut a, 100, 50);
        assert!(s.contains("ACTIVITY LOG"), "{s}");
        assert!(s.contains("something broke"));
        assert!(a.hits.iter().any(|h| h.key == IN_FILTER));
    }

    #[test]
    fn picker_modal_takes_over_input() {
        let (mut a, _rx) = app();
        a.role = Some(Role::Send);
        a.step = Step::Select;
        a.mode = TransferMode::Custom;
        let dir = std::env::temp_dir();
        a.picker = Some(super::super::picker::Picker::open(
            super::super::picker::PickKind::Many { dirs_only: false },
            super::super::picker::PickFor::Into(crate::ui::PickInto::Picks),
            Some(&dir.display().to_string()),
        ));
        a.focus = PK_LIST.into();
        let s = render(&mut a, 100, 30);
        assert!(s.contains("ADD FILES AND FOLDERS"), "{s}");
        assert!(a.hits.iter().all(|h| h.region == Region::Modal));
        assert!(a.hits.iter().any(|h| h.key == "pk:cancel"));
        click_key(&mut a, "pk:cancel");
        assert!(a.picker.is_none());
    }

    #[test]
    fn done_import_choices_click() {
        let (mut a, _rx) = app();
        a.role = Some(Role::Receive);
        a.step = Step::Done;
        a.received_dir = Some("/tmp/x/WINC Received/crossing-1".into());
        render(&mut a, 110, 40);
        click_key(&mut a, "imp:folder");
        assert!(a.use_folder);
        let s = render(&mut a, 110, 40);
        assert!(s.contains("No folder chosen yet"), "{s}");
        click_key(&mut a, "imp:choose");
        assert!(a.picker.is_some());
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let (mut a, _rx) = app();
        for (w, h) in [(1, 1), (10, 3), (30, 8), (200, 5)] {
            render(&mut a, w, h);
            a.role = Some(Role::Send);
            a.logs_open = true;
            render(&mut a, w, h);
            a.role = None;
            a.logs_open = false;
        }
    }
}
