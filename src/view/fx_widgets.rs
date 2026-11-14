// Compact FX controls embedded in the mixer's FX bus strips (the old Fx
// page is gone — the mixer is the only page). Each FX strip: M mute button
// (channel-strip style), preset dropdown, param1 knob, param2 toggle,
// routing checkboxes. FX1's strip also hosts the tap tempo button.
use crate::model::{
    channels::FxSlot,
    message::InterfaceMessage,
};
use crate::view::widgets::{format_percent, knob, sync_label};
use iced::{
    widget::{button, checkbox, column, container, pick_list, text, tooltip},
    Center, Element, Fill,
};

const MUTE_COLOR: iced::Color = iced::Color {
    r: 0.9,
    g: 0.2,
    b: 0.2,
    a: 1.0,
};

/// "M" mute button, same geometry as the channel strips' M buttons
/// (secondary, danger-tinted when muted). FX mute is global (both FX
/// processors). Lives in FX1's M/S band.
pub fn build_mute_btn(fx_muted: bool) -> Element<'static, InterfaceMessage> {
    let btn = button(text("M").size(12))
        .on_press(InterfaceMessage::FxMute)
        .padding([4, 10]);
    if fx_muted {
        btn.style(move |theme, status| {
            let mut style = button::danger(theme, status);
            style.background = Some(iced::Background::Color(MUTE_COLOR));
            style
        })
        .into()
    } else {
        btn.style(button::secondary).into()
    }
}

pub fn build_tap_tempo_btn(bpm: Option<u16>) -> Element<'static, InterfaceMessage> {
    let label = match bpm {
        Some(b) => format!("TAP {}", b),
        None => "TAP".to_string(),
    };
    // same snug geometry as the 48V button — it lives in that slot
    let tap_btn = button(text(label).size(11).center())
        .on_press(InterfaceMessage::TapTempo)
        .padding([3, 10])
        .style(button::secondary);

    tooltip(
        tap_btn,
        container(text("Tap repeatedly to set FX2 tempo (5-tap average).\nOnly works with delay/echo presets (1-12).").size(10))
            .padding(6)
            .style(container::rounded_box),
        tooltip::Position::Top,
    )
    .gap(4)
    .into()
}

/// pick_list item: carries the value (preset index, or param2 0/100),
/// displays the name.
#[derive(Clone, PartialEq)]
pub struct Choice {
    value: u8,
    name: &'static str,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

pub fn build_preset_pick(fx: &FxSlot) -> Element<'_, InterfaceMessage> {
    let fx_id = fx.id;
    let choices: Vec<Choice> = fx
        .presets()
        .iter()
        .enumerate()
        .map(|(i, p)| Choice {
            value: i as u8,
            name: p.name,
        })
        .collect();
    let selected = choices.get(fx.preset as usize).cloned();

    column![
        sync_label("Preset", 10.0, fx.preset_synced),
        pick_list(choices, selected, move |c| InterfaceMessage::FxPreset(fx_id, c.value))
            .text_size(9)
            .padding([2, 4])
            .width(Fill),
    ]
    .width(Fill)
    .spacing(2)
    .into()
}

pub fn build_param1_knob(fx: &FxSlot) -> Element<'_, InterfaceMessage> {
    let fx_id = fx.id;
    let info = fx.preset_info();
    column![
        sync_label(info.param1_label, 10.0, fx.param1_synced),
        knob(
            0..=100,
            fx.param1,
            50,
            28.0,
            move |v| InterfaceMessage::FxParam1(fx_id, v),
            format_percent(fx.param1),
        ),
    ]
    .align_x(Center)
    .spacing(2)
    .into()
}

pub fn build_param2_pick(
    fx_id: u8,
    off_label: &'static str,
    on_label: &'static str,
    is_on: bool,
    _is_synced: bool,
) -> Element<'static, InterfaceMessage> {
    // dropdown like the preset select: exactly one of the two options
    let choices = vec![
        Choice {
            value: 0,
            name: off_label,
        },
        Choice {
            value: 100,
            name: on_label,
        },
    ];
    let selected = choices[if is_on { 1 } else { 0 }].clone();

    pick_list(choices, Some(selected), move |c| {
        InterfaceMessage::FxParam2(fx_id, c.value)
    })
    .text_size(9)
    .padding([2, 4])
    .width(Fill)
    .into()
}

pub fn build_route_checkbox<'a>(
    fx: &'a FxSlot,
    which: usize,
    label: &'a str,
) -> Element<'a, InterfaceMessage> {
    let fx_id = fx.id;
    let (main, mon1, mon2) = (fx.route_main, fx.route_mon1, fx.route_mon2);
    let (checked, routing) = match which {
        0 => (main, (!main, mon1, mon2)),
        1 => (mon1, (main, !mon1, mon2)),
        _ => (mon2, (main, mon1, !mon2)),
    };
    checkbox::Checkbox::new(checked)
        .label(label)
        .on_toggle(move |_| InterfaceMessage::FxRouting(fx_id, routing.0, routing.1, routing.2))
        .text_size(9)
        .spacing(2)
        .into()
}
