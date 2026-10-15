//! Small building blocks shared by the screens: text helpers, the animated
//! cable, the waiting dots.

use super::style::{self, Pal, MONO};
use super::Message;
use iced::widget::canvas::{self, stroke, Frame, Geometry, LineCap, LineDash, Path, Stroke};
use iced::widget::{column, container, text, Text};
use iced::{mouse, window, Color, Element, Point, Rectangle, Renderer, Theme};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/* ---------- text ---------- */

/// Small uppercase mono label above a heading ("STEP 1 · PHYSICAL LINK").
pub fn eyebrow<'a>(p: Pal, s: impl Into<String>) -> Text<'a> {
    text(s.into().to_uppercase())
        .font(MONO)
        .size(style::S_SM)
        .style(style::color(p.slate))
}

pub fn mono<'a>(s: impl text::IntoFragment<'a>, size: f32, c: Color) -> Text<'a> {
    text(s).font(MONO).size(size).style(style::color(c))
}

pub fn body<'a>(s: impl text::IntoFragment<'a>, c: Color) -> Text<'a> {
    text(s).size(style::S_0).style(style::color(c))
}

pub fn warn_line<'a>(p: Pal, s: impl text::IntoFragment<'a>) -> Text<'a> {
    mono(s, style::S_SM, p.bad)
}

/// Step header: eyebrow, big heading, lead paragraph.
pub fn panel_head<'a>(
    p: Pal,
    eyebrow_text: impl Into<String>,
    heading: impl text::IntoFragment<'a>,
    lead: impl text::IntoFragment<'a>,
) -> Element<'a, Message> {
    column![
        eyebrow(p, eyebrow_text),
        text(heading).font(style::SEMI).size(style::S_3),
        container(body(lead, p.ink_2)).max_width(620),
    ]
    .spacing(6)
    .padding(iced::Padding::ZERO.bottom(14))
    .into()
}

/* ---------- animation ---------- */

// The waiting dots and the cable animate themselves: each frame they ask for the
// next redraw from inside the canvas, so the app's view isn't rebuilt 30
// times a second just to move a few pixels.
//
// Each also paints its own background over its whole canvas first. The CPU
// renderer only repaints what changed; without the fill, anti-aliased edges of
// earlier frames could survive and pile up into trails.

static START: LazyLock<Instant> = LazyLock::new(Instant::now);
/// ~30 fps for the flowing cable on a real GPU.
const FRAME: Duration = Duration::from_millis(33);
/// 4 fps when the CPU does the drawing (no GPU: Hyper-V / server VMs). Every
/// frame there costs CPU — WARP redraws the whole window — and a 30 fps
/// spinner alone took ~80% of the CPU on a Server 2019 VM.
const SOFTWARE_FRAME: Duration = Duration::from_millis(250);

/// Set at startup once iced reports the renderer (see `is_software`).
pub static SOFTWARE: AtomicBool = AtomicBool::new(false);

/// Is this renderer the CPU in disguise? The CPU fallback itself, or a
/// software GPU: Windows' WARP ("Microsoft Basic Render Driver"), Mesa's
/// llvmpipe, SwiftShader.
pub fn is_software(backend: &str, adapter: &str) -> bool {
    let b = backend.to_lowercase();
    let a = adapter.to_lowercase();
    b.contains("tiny-skia")
        || ["basic render", "warp", "llvmpipe", "swiftshader", "software"]
            .iter()
            .any(|k| a.contains(k))
}

fn software() -> bool {
    SOFTWARE.load(Ordering::Relaxed)
}

/// Seconds since the first animated frame; the phase of every animation.
#[derive(Default)]
pub struct Clock(f32);

/// Advance on each redraw and book the next one.
fn tick(clock: &mut Clock, event: &canvas::Event, frame: Duration) -> Option<canvas::Action<Message>> {
    match event {
        canvas::Event::Window(window::Event::RedrawRequested(now)) => {
            clock.0 = now.saturating_duration_since(*START).as_secs_f32();
            Some(canvas::Action::request_redraw_at(*now + frame))
        }
        _ => None,
    }
}

/* ---------- waiting dots ---------- */

/// One step of the dots: ".", "..", "...".
const DOT_STEP: Duration = Duration::from_millis(400);

/// "." → ".." → "..." on a `bg` background — the "still working" marker.
/// Replaced a spinning arc, which cost a VM with no GPU most of its CPU.
pub fn dots<'a>(color: Color, bg: Color) -> Element<'a, Message> {
    canvas::Canvas::new(Dots { color, bg }).width(22).height(18).into()
}

struct Dots {
    color: Color,
    bg: Color,
}

impl canvas::Program<Message> for Dots {
    type State = Clock;

    fn update(&self, clock: &mut Clock, event: &canvas::Event, _: Rectangle, _: mouse::Cursor) -> Option<canvas::Action<Message>> {
        tick(clock, event, DOT_STEP)
    }

    fn draw(&self, clock: &Clock, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        f.fill_rectangle(Point::ORIGIN, bounds.size(), self.bg);
        let n = (clock.0 / DOT_STEP.as_secs_f32()) as usize % 3 + 1;
        f.fill_text(canvas::Text {
            content: ".".repeat(n),
            position: Point::new(1.0, bounds.height / 2.0),
            color: self.color,
            size: iced::Pixels(style::S_1),
            font: style::BOLD,
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        vec![f.into_geometry()]
    }
}

/* ---------- cable ---------- */

const INSET: f32 = 8.0;

/// The signature element: idle → slack gray wire · linked → taut copper ·
/// flowing → cobalt dashes and pulses running along it.
pub struct Cable {
    pub pal: Pal,
    pub linked: bool,
    pub flowing: bool,
}

impl Cable {
    /// Point on the wire at `t` in 0..1 (quadratic sag when idle).
    fn at(&self, w: f32, h: f32, t: f32) -> Point {
        // inset so the glow and pulses stay inside the canvas
        let (x0, x1, y) = (INSET, w - INSET, h / 2.0);
        let sag = if self.linked { 0.0 } else { h * 0.3 };
        let x = x0 + (x1 - x0) * t;
        // quadratic Bézier with the control point at the middle, `sag` lower
        let yy = y + 2.0 * t * (1.0 - t) * sag;
        Point::new(x, yy)
    }
}

impl canvas::Program<Message> for Cable {
    type State = Clock;

    fn update(&self, clock: &mut Clock, event: &canvas::Event, _: Rectangle, _: mouse::Cursor) -> Option<canvas::Action<Message>> {
        // only the flowing wire moves
        if self.flowing {
            let frame = if software() { SOFTWARE_FRAME } else { FRAME };
            tick(clock, event, frame)
        } else {
            None
        }
    }

    fn draw(&self, clock: &Clock, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let p = self.pal;
        f.fill_rectangle(Point::ORIGIN, bounds.size(), p.paper);
        let wire = Path::new(|b| {
            b.move_to(self.at(w, h, 0.0));
            let sag = if self.linked { 0.0 } else { h * 0.3 };
            b.quadratic_curve_to(Point::new(w / 2.0, h / 2.0 + sag), self.at(w, h, 1.0));
        });

        let base = if self.linked { p.copper } else { p.hairline };
        if self.linked {
            // soft glow under the copper
            f.stroke(
                &wire,
                Stroke::default()
                    .with_width(8.0)
                    .with_color(Color { a: 0.18, ..p.copper })
                    .with_line_cap(LineCap::Round),
            );
        }
        f.stroke(
            &wire,
            Stroke::default().with_width(3.0).with_color(base).with_line_cap(LineCap::Round),
        );

        if self.flowing {
            let phase = clock.0;
            f.stroke(
                &wire,
                Stroke {
                    style: stroke::Style::Solid(p.cobalt),
                    width: 3.0,
                    line_cap: LineCap::Round,
                    line_dash: LineDash {
                        segments: &[5.0, 15.0],
                        offset: ((phase * 33.0) % 20.0) as usize,
                    },
                    ..Default::default()
                },
            );
            for k in 0..3 {
                let t = (phase / 1.1 + k as f32 / 3.0).fract();
                let at = self.at(w, h, t);
                f.fill(&Path::circle(at, 6.0), Color { a: 0.25, ..p.cobalt });
                f.fill(&Path::circle(at, 3.2), p.cobalt);
            }
        }
        vec![f.into_geometry()]
    }
}
