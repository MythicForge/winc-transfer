//! The TUI's widget kit: a top-to-bottom layout builder drawing into an
//! offscreen canvas (so any screen can scroll), and the hit map that makes
//! every button, row and field reachable by both mouse and keyboard.
//!
//! Each frame the screens register what they drew as `Hit`s, in reading
//! order. A click resolves through their screen rects; Tab / arrows walk the
//! same list; Enter / Space fires the focused one. Focus is a stable string
//! key, so it survives redraws.

use super::app::Msg;
use super::input::LineEdit;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Widget};
use std::borrow::Cow;
use std::cell::Cell;

thread_local! {
    /// Plain-ASCII glyphs: set when the locale isn't UTF-8 (common on minimal
    /// servers), where box drawing and arrows come out as mojibake. Per
    /// thread — only the UI thread draws, and tests stay independent.
    static ASCII: Cell<bool> = const { Cell::new(false) };
    /// `NO_COLOR` is set: modifiers (bold, reverse, underline) only.
    static NO_COLOR: Cell<bool> = const { Cell::new(false) };
}

pub fn set_ascii(on: bool) {
    ASCII.with(|c| c.set(on));
}

pub fn set_no_color(on: bool) {
    NO_COLOR.with(|c| c.set(on));
}

/// Tallest a scrolling canvas gets; anything past it is cut off.
pub const CANVAS_ROWS: u16 = 1200;

fn ascii() -> bool {
    ASCII.with(Cell::get)
}

/// `s` with every non-ASCII glyph swapped for an ASCII stand-in, when the
/// terminal needs it.
pub fn a(s: &str) -> Cow<'_, str> {
    if !ascii() || s.is_ascii() {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let r: &str = match c {
            '→' => "->",
            '←' => "<-",
            '↑' => "^",
            '↓' => "v",
            '·' | '•' => "*",
            '…' => "...",
            '—' => "--",
            '–' => "-",
            '✓' => "v",
            '⚠' => "!",
            '●' => "*",
            '○' => "o",
            '“' | '”' => "\"",
            '‘' | '’' => "'",
            '█' => "#",
            '░' => "-",
            '▸' => ">",
            '✕' | '×' => "x",
            '↻' => "",
            '▣' => "D",
            '▤' => "F",
            '━' | '─' => "-",
            '┃' | '│' => "|",
            c if c.is_ascii() => {
                out.push(c);
                continue;
            }
            _ => "?",
        };
        out.push_str(r);
    }
    Cow::Owned(out)
}

/// Display width in terminal columns.
pub fn width(s: &str) -> usize {
    Span::raw(s).width()
}

/// `s` cut to `max` columns by dropping its start, which for a path is the
/// least telling part: "…/scratchpad/e2e/importhere".
pub fn fit_left(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut w = 1; // the "…"
    for c in s.chars().rev() {
        let cw = width(c.encode_utf8(&mut [0; 4]));
        if w + cw > max {
            break;
        }
        w += cw;
        tail.push(c);
    }
    tail.reverse();
    format!("…{}", tail.into_iter().collect::<String>())
}

/// A label like "Use /some/long/path" cut to `max` columns, keeping the verb.
pub fn fit_label(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    match s.split_once(' ') {
        Some((verb, rest)) if width(verb) + 2 < max => format!("{verb} {}", fit_left(rest, max - width(verb) - 1)),
        _ => fit_left(s, max),
    }
}

/* ---------------- palette ---------------- */

fn fg(c: Color) -> Style {
    if NO_COLOR.with(Cell::get) {
        Style::new()
    } else {
        Style::new().fg(c)
    }
}

pub fn ink() -> Style {
    Style::new()
}
pub fn dim() -> Style {
    fg(Color::DarkGray)
}
pub fn soft() -> Style {
    fg(Color::Gray)
}
pub fn accent() -> Style {
    fg(Color::Cyan)
}
pub fn copper() -> Style {
    fg(Color::Yellow)
}
pub fn good() -> Style {
    fg(Color::Green)
}
pub fn bad() -> Style {
    fg(Color::Red)
}
pub fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}
pub fn focus_style() -> Style {
    Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

/* ---------------- hits ---------------- */

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Button,
    /// a checkbox / toggle row: Space and Enter both fire it
    Toggle,
    /// a text field: typed keys go to it, Enter submits
    Input,
    /// the file browser's list: arrows move inside it instead of between hits
    List,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    Top,
    Body,
    Log,
    Modal,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub key: String,
    pub on: Msg,
    pub kind: Kind,
    pub region: Region,
    /// where it was drawn on its canvas (for scrolling it into view)
    pub virt: Rect,
    /// where it is on screen, if it's visible at all
    pub screen: Option<Rect>,
}

/* ---------------- buttons ---------------- */

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BtnKind {
    Primary,
    Plain,
    Ghost,
    Danger,
}

pub struct Btn {
    pub key: String,
    pub label: String,
    pub kind: BtnKind,
    /// None = disabled: drawn dim, not clickable, skipped by Tab
    pub on: Option<Msg>,
}

pub fn btn(key: &str, kind: BtnKind, label: impl Into<String>, on: Option<Msg>) -> Btn {
    Btn {
        key: key.into(),
        label: label.into(),
        kind,
        on,
    }
}

/// One option of a segmented control.
pub struct Seg {
    pub key: &'static str,
    pub label: &'static str,
    pub on: bool,
    pub msg: Msg,
}

/* ---------------- the layout builder ---------------- */

pub struct Ui<'f> {
    pub buf: Buffer,
    /// left edge + width of the column being laid out (cards indent it)
    pub x: u16,
    pub w: u16,
    /// next free row
    pub y: u16,
    pub hits: Vec<Hit>,
    pub focus: &'f str,
    /// animation frame for the "..." busy dots
    pub tick: u64,
    /// the step's preferred first focus, used when nothing valid is focused
    pub autofocus: Option<String>,
}

impl<'f> Ui<'f> {
    pub fn new(w: u16, h: u16, focus: &'f str, tick: u64) -> Self {
        Self {
            buf: Buffer::empty(Rect::new(0, 0, w.max(1), h.max(1))),
            x: 0,
            w: w.max(1),
            y: 0,
            hits: Vec::new(),
            focus,
            tick,
            autofocus: None,
        }
    }

    pub fn focused(&self, key: &str) -> bool {
        self.focus == key
    }

    fn rows(&self) -> u16 {
        self.buf.area.height
    }

    /// Draw `s` at (x, y), clipped to the current column. Returns the columns used.
    pub fn put(&mut self, x: u16, y: u16, s: &str, st: Style) -> u16 {
        let right = self.x + self.w;
        if y >= self.rows() || x >= right {
            return 0;
        }
        let s = a(s);
        let (end, _) = self.buf.set_stringn(x, y, s.as_ref(), (right - x) as usize, st);
        end - x
    }

    /// `put`, cut to at most `n` columns.
    pub fn put_n(&mut self, x: u16, y: u16, s: &str, st: Style, n: u16) -> u16 {
        let saved = self.w;
        self.w = (x + n).saturating_sub(self.x).min(saved);
        let c = self.put(x, y, s, st);
        self.w = saved;
        c
    }

    /// Register something clickable/focusable at `rect` (canvas coordinates).
    pub fn hit(&mut self, key: &str, rect: Rect, on: Msg, kind: Kind) {
        self.hits.push(Hit {
            key: key.into(),
            on,
            kind,
            region: Region::Body,
            virt: rect,
            screen: None,
        });
    }

    pub fn gap(&mut self) {
        self.y += 1;
    }

    /// One row of styled pieces, no wrapping (cut at the edge).
    pub fn line(&mut self, segs: &[(&str, Style)]) {
        let mut x = self.x;
        for (s, st) in segs {
            x += self.put(x, self.y, s, *st);
        }
        self.y += 1;
    }

    /// A wrapped paragraph.
    pub fn text(&mut self, s: &str, st: Style) {
        self.rich(&[(s, st)]);
    }

    /// A wrapped paragraph of mixed styles.
    pub fn rich(&mut self, segs: &[(&str, Style)]) {
        self.rich_at(segs, 0);
    }

    /// Wrapped, with continuation lines indented by `hang` columns.
    fn rich_at(&mut self, segs: &[(&str, Style)], hang: u16) {
        let owned: Vec<(String, Style)> = segs.iter().map(|(s, st)| (a(s).into_owned(), *st)).collect();
        let first_w = self.w as usize;
        let lines = wrap(&owned, first_w.saturating_sub(hang as usize).max(1));
        for (i, l) in lines.iter().enumerate() {
            let mut x = self.x + if i == 0 { 0 } else { hang };
            for (s, st) in l {
                x += self.put(x, self.y, s, *st);
            }
            self.y += 1;
        }
    }

    /// Eyebrow, title and a blurb: the head of every step.
    pub fn head(&mut self, eyebrow: &str, title: &str, blurb: &str) {
        self.line(&[(&eyebrow.to_uppercase(), dim())]);
        self.line(&[(title, bold())]);
        self.text(blurb, soft());
        self.gap();
    }

    pub fn eyebrow(&mut self, s: &str) {
        self.line(&[(&s.to_uppercase(), dim())]);
    }

    pub fn bullets(&mut self, items: &[&str]) {
        for s in items {
            let x = self.x;
            self.put(x, self.y, "• ", dim());
            let (ox, ow) = (self.x, self.w);
            self.x += 2;
            self.w = self.w.saturating_sub(2).max(1);
            self.text(s, soft());
            self.x = ox;
            self.w = ow;
        }
    }

    pub fn warn(&mut self, s: &str) {
        self.rich_at(&[("⚠ ", copper()), (s, copper())], 2);
    }

    /// A status line: animated dots while busy, a dot once settled.
    pub fn status(&mut self, busy: bool, dot: Style, s: &str) {
        let lead = if busy {
            [".  ", ".. ", "..."][(self.tick % 3) as usize]
        } else {
            "●  "
        };
        let st = if busy { accent() } else { dot };
        self.rich_at(&[(lead, st), (s, ink())], 3);
    }

    pub fn rule(&mut self) {
        let line = "─".repeat(self.w as usize);
        let (x, y) = (self.x, self.y);
        self.put(x, y, &line, dim());
        self.y += 1;
    }

    /// Buttons left to right, wrapping onto a new row when they run out of room.
    pub fn buttons(&mut self, items: Vec<Btn>) {
        let mut x = self.x;
        for b in items {
            let label = format!(" {} ", a(&b.label));
            let wd = width(&label) as u16 + 2;
            if x > self.x && x + wd > self.x + self.w {
                self.y += 1;
                x = self.x;
            }
            self.button_at(x, self.y, &b);
            x += wd + 2;
        }
        self.y += 1;
    }

    /// One button at (x, y); returns its width.
    pub fn button_at(&mut self, x: u16, y: u16, b: &Btn) -> u16 {
        let label = format!(" {} ", a(&b.label));
        let focused = b.on.is_some() && self.focused(&b.key);
        let base = match (b.on.is_some(), b.kind) {
            (false, _) => dim(),
            (true, BtnKind::Primary) => accent().add_modifier(Modifier::BOLD),
            (true, BtnKind::Danger) => bad().add_modifier(Modifier::BOLD),
            (true, BtnKind::Ghost) => soft(),
            (true, BtnKind::Plain) => ink(),
        };
        let st = if focused { base.patch(focus_style()) } else { base };
        let mut w = self.put(x, y, "[", base);
        w += self.put(x + w, y, &label, st);
        w += self.put(x + w, y, "]", base);
        if let Some(on) = &b.on {
            self.hit(&b.key, Rect::new(x, y, w, 1), on.clone(), Kind::Button);
        }
        w
    }

    /// A segmented control: ( ● One )( ○ Two ).
    pub fn seg(&mut self, items: Vec<Seg>) {
        let mut x = self.x;
        for s in items {
            let label = format!(" {} {} ", if s.on { "●" } else { "○" }, s.label);
            let label = a(&label).into_owned();
            let wd = width(&label) as u16;
            if x > self.x && x + wd > self.x + self.w {
                self.y += 1;
                x = self.x;
            }
            let mut st = if s.on { accent().add_modifier(Modifier::BOLD) } else { soft() };
            if self.focused(s.key) {
                st = st.patch(focus_style());
            }
            self.put(x, self.y, &label, st);
            self.hit(s.key, Rect::new(x, self.y, wd, 1), s.msg, Kind::Button);
            x += wd + 1;
        }
        self.y += 1;
    }

    /// A one-line text field, `w` columns wide (0 = the whole column).
    pub fn input(&mut self, key: &str, e: &LineEdit, placeholder: &str, submit: Msg, w: u16) {
        let w = if w == 0 { self.w } else { w.min(self.w) };
        let (x, y) = (self.x, self.y);
        let focused = self.focused(key);
        let lead_st = if focused { accent().add_modifier(Modifier::BOLD) } else { dim() };
        let lead = self.put(x, y, "› ", lead_st);
        let room = w.saturating_sub(lead + 1).max(1) as usize;
        let field = Style::new().add_modifier(Modifier::UNDERLINED);
        // blank, underlined field
        self.put(x + lead, y, &" ".repeat(room + 1), field);
        if e.text.is_empty() && !focused {
            self.put(x + lead, y, placeholder, dim().add_modifier(Modifier::UNDERLINED));
        } else {
            // keep the cursor in view on long values
            let chars: Vec<char> = e.text.chars().collect();
            let start = e.cursor.saturating_sub(room.saturating_sub(1));
            let shown: String = chars[start.min(chars.len())..].iter().collect();
            self.put(x + lead, y, &shown, field);
            if focused {
                let cx = x + lead + width(&chars[start.min(chars.len())..e.cursor.min(chars.len())].iter().collect::<String>()) as u16;
                if let Some(c) = self.buf.cell_mut((cx, y)) {
                    c.set_style(Style::new().add_modifier(Modifier::REVERSED));
                }
            }
        }
        self.hit(key, Rect::new(x, y, w, 1), submit, Kind::Input);
        self.y += 1;
    }

    /// Content inside a rounded border, with an optional title in the top edge.
    pub fn card(&mut self, title: Option<&str>, f: impl FnOnce(&mut Ui)) {
        let top = self.y;
        let (ox, ow) = (self.x, self.w);
        self.x += 2;
        self.w = self.w.saturating_sub(4).max(1);
        self.y += 1;
        f(self);
        self.x = ox;
        self.w = ow;
        let bottom = (self.y + 1).min(self.rows());
        let area = Rect::new(ox, top, ow, bottom.saturating_sub(top).max(2));
        let mut block = Block::new().borders(Borders::ALL).border_style(dim());
        block = if ascii() {
            block.border_set(border::Set {
                top_left: "+",
                top_right: "+",
                bottom_left: "+",
                bottom_right: "+",
                vertical_left: "|",
                vertical_right: "|",
                horizontal_top: "-",
                horizontal_bottom: "-",
            })
        } else {
            block.border_type(BorderType::Rounded)
        };
        if let Some(t) = title {
            block = block.title(Span::styled(format!(" {} ", t.to_uppercase()), soft()));
        }
        let area = area.intersection(self.buf.area);
        block.render(area, &mut self.buf);
        self.y = bottom;
    }

    /// A progress bar with the percentage after it.
    pub fn bar(&mut self, pct: f64) {
        let pct = pct.clamp(0.0, 100.0);
        let label = format!(" {pct:>3.0}%");
        let room = self.w.saturating_sub(label.len() as u16) as usize;
        let full = ((pct / 100.0) * room as f64).round() as usize;
        let (x, y) = (self.x, self.y);
        let mut c = self.put(x, y, &"█".repeat(full), accent());
        c += self.put(x + c, y, &"░".repeat(room - full.min(room)), dim());
        self.put(x + c, y, &label, bold());
        self.y += 1;
    }
}

/// Word-wrap styled pieces to `width` columns. A word longer than a whole
/// line (a long path) is split mid-word; '\n' forces a break.
pub fn wrap(segs: &[(String, Style)], width_cols: usize) -> Vec<Vec<(String, Style)>> {
    let width_cols = width_cols.max(1);
    let mut lines: Vec<Vec<(String, Style)>> = vec![Vec::new()];
    let mut col = 0usize;
    let push = |lines: &mut Vec<Vec<(String, Style)>>, s: &str, st: Style| {
        let l = lines.last_mut().unwrap();
        match l.last_mut() {
            Some((t, s2)) if *s2 == st => t.push_str(s),
            _ => l.push((s.to_string(), st)),
        }
    };
    for (text, st) in segs {
        for (pi, para) in text.split('\n').enumerate() {
            if pi > 0 {
                lines.push(Vec::new());
                col = 0;
            }
            for tok in para.split_inclusive(' ') {
                let word = tok.trim_end_matches(' ');
                let ww = width(word);
                if col > 0 && col + ww > width_cols {
                    lines.push(Vec::new());
                    col = 0;
                }
                if ww > width_cols {
                    // hard-split
                    for ch in tok.chars() {
                        let cw = width(ch.encode_utf8(&mut [0; 4]));
                        if col + cw > width_cols && col > 0 {
                            lines.push(Vec::new());
                            col = 0;
                        }
                        push(&mut lines, ch.encode_utf8(&mut [0; 4]), *st);
                        col += cw;
                    }
                    continue;
                }
                if col == 0 && word.is_empty() {
                    // don't start a line with the space that broke it
                    continue;
                }
                push(&mut lines, tok, *st);
                col += width(tok);
            }
        }
    }
    lines
}

/// The receiver's pairing code, three rows tall, so it reads across a room.
/// None when the terminal can't draw the glyphs (ASCII mode).
pub fn big_digits(code: &str) -> Option<[String; 3]> {
    if ascii() {
        return None;
    }
    const D: [[&str; 3]; 10] = [
        ["┏━┓", "┃ ┃", "┗━┛"],
        ["  ┃", "  ┃", "  ┃"],
        ["━━┓", "┏━┛", "┗━━"],
        ["━━┓", " ━┫", "━━┛"],
        ["┃ ┃", "┗━┫", "  ┃"],
        ["┏━━", "┗━┓", "━━┛"],
        ["┏━━", "┣━┓", "┗━┛"],
        ["━━┓", "  ┃", "  ┃"],
        ["┏━┓", "┣━┫", "┗━┛"],
        ["┏━┓", "┗━┫", "━━┛"],
    ];
    let mut out = [String::new(), String::new(), String::new()];
    for c in code.chars() {
        match c.to_digit(10) {
            Some(d) => {
                for (r, row) in out.iter_mut().enumerate() {
                    row.push_str(D[d as usize][r]);
                    row.push(' ');
                }
            }
            None => out.iter_mut().for_each(|r| r.push_str("  ")),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Vec<(String, Style)>]) -> Vec<String> {
        lines.iter().map(|l| l.iter().map(|(s, _)| s.as_str()).collect()).collect()
    }

    #[test]
    fn wraps_words_and_long_paths() {
        let st = Style::new();
        // a trailing space may overhang; drawing clips it
        let got = plain(&wrap(&[("the quick brown fox".into(), st)], 9));
        assert_eq!(got, ["the quick ", "brown fox"]);
        let got = plain(&wrap(&[("the quick brown fox".into(), st)], 8));
        assert_eq!(got, ["the ", "quick ", "brown ", "fox"]);
        let got = plain(&wrap(&[("/a/very/long/path".into(), st)], 6));
        assert_eq!(got, ["/a/ver", "y/long", "/path"]);
        let got = plain(&wrap(&[("one\ntwo".into(), st)], 20));
        assert_eq!(got, ["one", "two"]);
        // styles split across a line stay separate pieces
        let got = wrap(&[("go to ".into(), st), ("/x".into(), bold())], 20);
        assert_eq!(got[0].len(), 2);
    }

    #[test]
    fn fitting() {
        assert_eq!(fit_left("/a/b/c", 10), "/a/b/c");
        assert_eq!(fit_left("/home/user/importhere", 12), "…/importhere");
        assert_eq!(fit_label("Use /home/user/importhere", 16), "Use …/importhere");
        assert_eq!(fit_label("Add 2 marked items", 40), "Add 2 marked items");
        assert_eq!(width(&fit_label("Use /x/y/zzzzzzzzzzzzzzzz", 12)), 12);
    }

    #[test]
    fn ascii_fallback() {
        set_ascii(true);
        assert_eq!(a("Pair → · ✓"), "Pair -> * v");
        assert_eq!(a("plain"), "plain");
        assert!(big_digits("123").is_none());
        set_ascii(false);
        assert_eq!(big_digits("10").unwrap()[0], "  ┃ ┏━┓ ");
    }

    #[test]
    fn buttons_register_hits_unless_disabled() {
        let mut ui = Ui::new(40, 10, "b", 0);
        ui.buttons(vec![
            btn("a", BtnKind::Plain, "One", Some(Msg::Noop)),
            btn("b", BtnKind::Primary, "Two", None),
            btn("c", BtnKind::Ghost, "Three", Some(Msg::Noop)),
        ]);
        let keys: Vec<&str> = ui.hits.iter().map(|h| h.key.as_str()).collect();
        assert_eq!(keys, ["a", "c"]);
        assert_eq!(ui.hits[0].virt, Rect::new(0, 0, 7, 1));
        assert_eq!(ui.y, 1);
    }

    #[test]
    fn buttons_wrap_when_narrow() {
        let mut ui = Ui::new(14, 10, "", 0);
        ui.buttons(vec![
            btn("a", BtnKind::Plain, "One", Some(Msg::Noop)),
            btn("b", BtnKind::Plain, "Two", Some(Msg::Noop)),
        ]);
        assert_eq!(ui.hits[1].virt.y, 1);
        assert_eq!(ui.y, 2);
    }
}
