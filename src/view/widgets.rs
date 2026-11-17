use crate::model::message::InterfaceMessage;
use iced::{
    mouse,
    widget::canvas::{
        self, Canvas, Frame, Geometry, Path, Stroke,
    },
    widget::{container, mouse_area, row, slider, text, tooltip, vertical_slider, Row},
    Center, Element, Length, Point, Rectangle,
};
use std::ops::RangeInclusive;

pub const UNSYNCED_COLOR: iced::Color = iced::Color {
    r: 1.0,
    g: 0.7,
    b: 0.15,
    a: 1.0,
};

pub fn sync_label<'a>(label: &'a str, size: f32, is_synced: bool) -> Row<'a, InterfaceMessage> {
    if is_synced {
        row![text(label).size(size)].align_y(Center).spacing(3)
    } else {
        row![
            text("\u{25CF}").size(7).color(UNSYNCED_COLOR),
            text(label).size(size),
        ]
        .align_y(Center)
        .spacing(3)
    }
}

pub fn sync_dot(is_synced: bool) -> Element<'static, InterfaceMessage> {
    if is_synced {
        text("").size(7).into()
    } else {
        text("\u{25CF}").size(7).color(UNSYNCED_COLOR).into()
    }
}

#[allow(dead_code)] // kept for the vertical variant's symmetry; CI -D warnings
pub fn h_slider<'a, F>(
    range: RangeInclusive<u8>,
    value: u8,
    on_change: F,
    tip: String,
) -> Element<'a, InterfaceMessage>
where
    F: Fn(u8) -> InterfaceMessage + 'a,
{
    tooltip(
        slider(range, value, on_change).width(iced::Fill),
        container(text(tip).size(10))
            .padding(4)
            .style(container::rounded_box),
        tooltip::Position::Top,
    )
    .gap(4)
    .into()
}

// level fader scale is 1..=127 over −70…+10 dB (see sysex_parser::db_to_cc) → 0 dB = 111
pub const LEVEL_ZERO_DB: u8 = 111;

/// Wrap a widget so right-click sends one message (faders: snap to 0 dB).
pub fn on_right_click<'a>(
    content: impl Into<Element<'a, InterfaceMessage>>,
    msg: InterfaceMessage,
) -> Element<'a, InterfaceMessage> {
    mouse_area(content).on_right_press(msg).into()
}

pub fn v_slider<'a, F>(
    range: RangeInclusive<u8>,
    value: u8,
    height: f32,
    on_change: F,
    tip: String,
) -> Element<'a, InterfaceMessage>
where
    F: Fn(u8) -> InterfaceMessage + 'a,
{
    tooltip(
        vertical_slider(range, value, on_change).height(height),
        container(text(tip).size(10))
            .padding(4)
            .style(container::rounded_box),
        tooltip::Position::Right,
    )
    .gap(4)
    .into()
}

// ── rotary knob (canvas): relative drag + wheel fine-tune ─────────────

const KNOB_SWEEP_DEG: f32 = 270.0; // −135° … +135° around 12 o'clock
const KNOB_DRAG_PIXELS: f32 = 160.0; // mouse travel for the full range

#[derive(Default)]
pub struct KnobDrag {
    active: bool,
    start_y: f32,
    start_value: u8,
}

pub struct KnobProgram<F> {
    min: u8,
    max: u8,
    value: u8,
    default: u8,
    on_change: F,
}

impl<F> canvas::Program<InterfaceMessage> for KnobProgram<F>
where
    F: Fn(u8) -> InterfaceMessage,
{
    type State = KnobDrag;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<InterfaceMessage>> {
        let canvas::Event::Mouse(mouse_event) = event else {
            return None;
        };
        match mouse_event {
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                if cursor.is_over(bounds) {
                    state.active = true;
                    state.start_y = cursor.position().map(|p| p.y).unwrap_or(0.0);
                    state.start_value = self.value;
                    Some(canvas::Action::capture())
                } else {
                    None
                }
            }
            // right-click: reset to default
            mouse::Event::ButtonPressed(mouse::Button::Right) => {
                if cursor.is_over(bounds) && self.value != self.default {
                    Some(canvas::Action::publish((self.on_change)(self.default)).and_capture())
                } else {
                    None
                }
            }
            // relative drag: no jump on grab, keeps working outside the widget
            mouse::Event::CursorMoved { .. } => {
                if !state.active {
                    return None;
                }
                let Some(pos) = cursor.position() else {
                    return Some(canvas::Action::capture());
                };
                let dy = state.start_y - pos.y; // up = louder
                let span = (self.max - self.min) as f32;
                let v = state.start_value as f32 + dy / KNOB_DRAG_PIXELS * span;
                let v = v.round().clamp(self.min as f32, self.max as f32) as u8;
                if v != self.value {
                    Some(canvas::Action::publish((self.on_change)(v)).and_capture())
                } else {
                    Some(canvas::Action::capture())
                }
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                if state.active {
                    state.active = false;
                    Some(canvas::Action::capture())
                } else {
                    None
                }
            }
            mouse::Event::WheelScrolled { delta } => {
                if !cursor.is_over(bounds) {
                    return None;
                }
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 20.0,
                };
                if y == 0.0 {
                    return None;
                }
                let step = if y > 0.0 { 1 } else { -1 };
                let v = (self.value as i32 + step).clamp(self.min as i32, self.max as i32) as u8;
                if v != self.value {
                    Some(canvas::Action::publish((self.on_change)(v)).and_capture())
                } else {
                    Some(canvas::Action::capture())
                }
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry<iced::Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let radius = bounds.width / 2.0 - 2.0;

        // dial body
        frame.fill(
            &Path::circle(center, radius),
            iced::Color::from_rgb(0.16, 0.16, 0.19),
        );

        // value arc (accent), from −135° to current angle (y-down: 12 o'clock = −90°)
        let frac =
            (self.value - self.min) as f32 / (self.max - self.min).max(1) as f32;
        let angle_at = |f: f32| (-135.0 + f * KNOB_SWEEP_DEG - 90.0).to_radians();
        let start = angle_at(0.0);
        let end = angle_at(frac);
        let arc_radius = radius - 3.0;
        let mut builder = canvas::path::Builder::new();
        let steps = 24;
        for i in 0..=steps {
            let a = start + (end - start) * (i as f32 / steps as f32);
            let p = Point::new(
                center.x + arc_radius * a.cos(),
                center.y + arc_radius * a.sin(),
            );
            if i == 0 {
                builder.move_to(p);
            } else {
                builder.line_to(p);
            }
        }
        let accent = if cursor.is_over(bounds) {
            iced::Color::from_rgb(0.35, 0.85, 0.5)
        } else {
            iced::Color::from_rgb(0.55, 0.75, 0.95)
        };
        frame.stroke(
            &builder.build(),
            Stroke::default().with_color(accent).with_width(2.5),
        );

        // pointer line
        let inner = radius * 0.25;
        let outer = radius * 0.8;
        let p1 = Point::new(center.x + inner * end.cos(), center.y + inner * end.sin());
        let p2 = Point::new(center.x + outer * end.cos(), center.y + outer * end.sin());
        let mut pointer = canvas::path::Builder::new();
        pointer.move_to(p1);
        pointer.line_to(p2);
        frame.stroke(
            &pointer.build(),
            Stroke::default().with_color(iced::Color::from_rgb(0.9, 0.9, 0.9)).with_width(2.0),
        );

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.active {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }
}

/// rotary knob widget: drag sets value relatively, wheel fine-tunes, right-click resets
pub fn knob<'a, F>(
    range: RangeInclusive<u8>,
    value: u8,
    default: u8,
    size: f32,
    on_change: F,
    tip: String,
) -> Element<'a, InterfaceMessage>
where
    F: Fn(u8) -> InterfaceMessage + 'a,
{
    let program = KnobProgram {
        min: *range.start(),
        max: *range.end(),
        value,
        default,
        on_change,
    };
    tooltip(
        Canvas::new(program)
            .width(Length::Fixed(size))
            .height(Length::Fixed(size)),
        container(text(tip).size(10))
            .padding(4)
            .style(container::rounded_box),
        tooltip::Position::Top,
    )
    .gap(3)
    .into()
}

// ── CC → display value conversions ────────────────────────────────────
// Ranges from FLOW 8 MIDI Implementation Chart

const LEVEL_DB_MIN: f32 = -70.0;
const LEVEL_DB_MAX: f32 = 10.0;
const GAIN_MIN: f32 = -20.0;
const GAIN_MAX: f32 = 60.0;
const EQ_MIN: f32 = -15.0;
const EQ_MAX: f32 = 15.0;
const PAN_MIN: f32 = -1.0;
const PAN_MAX: f32 = 1.0;
const LOWCUT_MIN_HZ: f32 = 20.0;
const LOWCUT_MAX_HZ: f32 = 600.0;

fn cc_to_range(cc: u8, cc_max: u8, min: f32, max: f32) -> f32 {
    min + (cc as f32 / cc_max as f32) * (max - min)
}

pub fn format_level(cc: u8) -> String {
    if cc == 0 {
        return "OFF".to_string();
    }
    let db = LEVEL_DB_MIN + ((cc as f32 - 1.0) / 126.0) * (LEVEL_DB_MAX - LEVEL_DB_MIN);
    format!("{:.1} dB", db)
}

pub fn format_gain(cc: u8) -> String {
    let db = cc_to_range(cc, 127, GAIN_MIN, GAIN_MAX);
    format!("{:.1} dB", db)
}

pub fn format_pan(cc: u8) -> String {
    let pan = cc_to_range(cc, 127, PAN_MIN, PAN_MAX);
    if pan.abs() < 0.02 {
        "C".to_string()
    } else if pan < 0.0 {
        format!("L {:.0}%", pan.abs() * 100.0)
    } else {
        format!("R {:.0}%", pan * 100.0)
    }
}

pub fn format_eq(cc: u8) -> String {
    let db = cc_to_range(cc, 127, EQ_MIN, EQ_MAX);
    if db.abs() < 0.3 {
        "0 dB".to_string()
    } else {
        format!("{:+.1} dB", db)
    }
}

pub fn format_lowcut(cc: u8) -> String {
    let hz = cc_to_range(cc, 127, LOWCUT_MIN_HZ, LOWCUT_MAX_HZ);
    format!("{:.0} Hz", hz)
}

const LIMITER_MIN: f32 = -30.0;
const LIMITER_MAX: f32 = 0.0;

pub fn format_comp(cc: u8) -> String {
    let pct = cc.min(100);
    format!("{}%", pct)
}

pub fn format_send(cc: u8) -> String {
    format_level(cc)
}

pub fn format_percent(cc: u8) -> String {
    format!("{}%", cc)
}

pub fn format_limiter(cc: u8) -> String {
    let db = cc_to_range(cc, 127, LIMITER_MIN, LIMITER_MAX);
    if db.abs() < 0.3 {
        "0 dB".to_string()
    } else {
        format!("{:.1} dB", db)
    }
}
