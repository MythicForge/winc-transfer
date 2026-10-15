//! "Signal Bench" look, ported from the old tokens.css / app.css.
//! Two accents carry meaning: cobalt = data/signal, copper = the physical cable.

use iced::widget::{button, container, text, text_input};
use iced::{border, Background, Border, Color, Font, Shadow, Theme};

#[derive(Clone, Copy, Debug)]
pub struct Pal {
    pub paper: Color,
    pub paper_2: Color,
    pub panel: Color,
    pub ink: Color,
    pub ink_2: Color,
    pub slate: Color,
    pub hairline: Color,
    pub cobalt: Color,
    pub cobalt_soft: Color,
    pub copper: Color,
    pub copper_soft: Color,
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
}

const fn hex(rgb: u32) -> Color {
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const LIGHT: Pal = Pal {
    paper: hex(0xf5f7fa),
    paper_2: hex(0xeef1f6),
    panel: hex(0xffffff),
    ink: hex(0x1a1f2b),
    ink_2: hex(0x3a4256),
    slate: hex(0x8a93a6),
    hairline: hex(0xd7dce6),
    cobalt: hex(0x2e5bff),
    cobalt_soft: hex(0xe5ebff),
    copper: hex(0xc6683b),
    copper_soft: hex(0xf6e6dd),
    good: hex(0x1f9d6b),
    warn: hex(0xc99a1e),
    bad: hex(0xd1443b),
};

pub const DARK: Pal = Pal {
    paper: hex(0x10131a),
    paper_2: hex(0x0b0e14),
    panel: hex(0x171b24),
    ink: hex(0xeef1f6),
    ink_2: hex(0xb7bfce),
    slate: hex(0x808a9d),
    hairline: hex(0x262c38),
    cobalt: hex(0x6c8bff),
    cobalt_soft: hex(0x1b2338),
    copper: hex(0xe08a5c),
    copper_soft: hex(0x2a1c14),
    good: hex(0x1f9d6b),
    warn: hex(0xc99a1e),
    bad: hex(0xe0574e),
};

/// The iced theme behind the stock widgets (scrollbars, checkboxes, caret).
pub fn theme(p: &Pal, dark: bool) -> Theme {
    Theme::custom(
        if dark { "WINCI dark" } else { "WINCI light" },
        iced::theme::Palette {
            background: p.paper,
            text: p.ink,
            primary: p.cobalt,
            success: p.good,
            warning: p.warn,
            danger: p.bad,
        },
    )
}

/* ---------- type ---------- */

/// Text sizes (the old CSS step scale, 1rem = 16px).
pub const S_SM: f32 = 13.0;
pub const S_0: f32 = 15.0;
pub const S_1: f32 = 18.0;
pub const S_2: f32 = 24.0;
pub const S_3: f32 = 34.0;
pub const S_4: f32 = 46.0;

pub const RADIUS: f32 = 10.0;
pub const RADIUS_SM: f32 = 6.0;

#[cfg(windows)]
const SANS: &str = "Segoe UI";
#[cfg(not(windows))]
const SANS: &str = "Noto Sans";

pub const BODY: Font = Font::with_name(SANS);
pub const BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..BODY
};
pub const SEMI: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..BODY
};
#[cfg(windows)]
pub const MONO: Font = Font::with_name("Consolas");
#[cfg(not(windows))]
pub const MONO: Font = Font::MONOSPACE;

/* ---------- containers ---------- */

// No drop shadows anywhere: the CPU renderer repaints only what changed and
// doesn't count a shadow's blur outside its widget, so shadows leave ghosts
// behind when content moves. Hairline borders carry the depth instead.

/// White card with a hairline border and a soft shadow.
pub fn card(p: Pal) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(p.panel.into()),
        border: border::rounded(RADIUS).width(1).color(p.hairline),
        ..Default::default()
    }
}

/// Card that gets a cobalt accent border when `on` (the cable's end nodes).
pub fn node(p: Pal, on: bool) -> impl Fn(&Theme) -> container::Style {
    move |t| {
        if !on {
            return card(p)(t);
        }
        container::Style {
            background: Some(p.panel.into()),
            border: border::rounded(RADIUS).width(2).color(p.cobalt),
            ..Default::default()
        }
    }
}

/// Smaller list row (sources, adapters).
pub fn row_box(p: Pal, on: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(if on { p.cobalt_soft } else { p.panel }.into()),
        border: border::rounded(RADIUS_SM)
            .width(1)
            .color(if on { p.cobalt } else { p.hairline }),
        ..Default::default()
    }
}

pub fn fill(bg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(bg.into()),
        ..Default::default()
    }
}

/// Rounded tinted box: copper caveats, the code box, chips.
pub fn tinted(bg: Color, line: Color, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(bg.into()),
        border: border::rounded(radius).width(1).color(line),
        ..Default::default()
    }
}

/// Outline-only badge ("browser", "imported").
pub fn badge(c: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(c),
        border: border::rounded(4).width(1).color(c),
        ..Default::default()
    }
}

/// 1px rule.
pub fn rule(p: Pal) -> impl Fn(&Theme) -> container::Style {
    fill(p.hairline)
}

/* ---------- buttons ---------- */

#[derive(Clone, Copy, PartialEq)]
pub enum Btn {
    /// Cobalt, filled.
    Primary,
    /// Panel with a border.
    Plain,
    /// Text only until hovered.
    Ghost,
    /// Small red outline ("Overwrite?").
    Danger,
}

pub fn btn(p: Pal, kind: Btn) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let disabled = status == button::Status::Disabled;
        let (bg, fg, line) = match kind {
            Btn::Primary => (
                Some(if hovered { mix(p.cobalt, Color::BLACK, 0.12) } else { p.cobalt }),
                Color::WHITE,
                Color::TRANSPARENT,
            ),
            Btn::Plain => (
                Some(if hovered { p.paper_2 } else { p.panel }),
                p.ink,
                if hovered { p.slate } else { p.hairline },
            ),
            Btn::Ghost => (
                hovered.then_some(p.paper_2),
                p.ink_2,
                if hovered { p.hairline } else { Color::TRANSPARENT },
            ),
            Btn::Danger => (hovered.then_some(p.paper_2), p.bad, p.bad),
        };
        let mut s = button::Style {
            background: bg.map(Background::from),
            text_color: fg,
            border: border::rounded(RADIUS_SM).width(1).color(line),
            shadow: Shadow::default(),
            snap: true,
        };
        if disabled {
            s.text_color = Color { a: 0.45, ..fg };
            s.background = s.background.map(|b| b.scale_alpha(0.45));
            s.border.color = Color { a: 0.45 * line.a, ..line };
        }
        s
    }
}

/// Segmented-control half: raised panel when on.
pub fn seg(p: Pal, on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: on.then_some(p.panel.into()),
        text_color: if on || status == button::Status::Hovered { p.ink } else { p.ink_2 },
        border: border::rounded(5).width(1).color(if on { p.hairline } else { Color::TRANSPARENT }),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Big selectable card (transfer mode, source rows).
pub fn tile(p: Pal, on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: Some(if on { p.cobalt_soft } else { p.panel }.into()),
            text_color: p.ink,
            border: border::rounded(RADIUS_SM).width(1).color(if on {
                p.cobalt
            } else if hovered {
                p.slate
            } else {
                p.hairline
            }),
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

/* ---------- inputs ---------- */

pub fn input(p: Pal) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_, status| {
        let focused = matches!(status, text_input::Status::Focused { .. });
        text_input::Style {
            background: p.panel.into(),
            border: Border {
                color: if focused { p.cobalt } else { p.hairline },
                width: if focused { 1.5 } else { 1.0 },
                radius: RADIUS.into(),
            },
            icon: p.slate,
            placeholder: p.slate,
            value: p.ink,
            selection: Color { a: 0.35, ..p.cobalt },
        }
    }
}

/* ---------- text ---------- */

pub fn color(c: Color) -> impl Fn(&Theme) -> text::Style {
    move |_| text::Style { color: Some(c) }
}

/// Blend `a` toward `b` by `t`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}
