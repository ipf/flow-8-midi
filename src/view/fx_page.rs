// Compact FX controls embedded in the mixer's FX bus strips (the old Fx
// page is gone — the mixer is the only page). Each FX strip: 2×8 preset
// grid, param1 slider, param2 toggle, routing checkboxes; FX1's strip
// also hosts the global FX mute + tap tempo.
use crate::model::{
    channels::FxSlot,
    message::InterfaceMessage,
};
use crate::view::widgets::{sync_label, UNSYNCED_COLOR};
use iced::{
    widget::{button, checkbox, column, container, row, slider, text, tooltip, Column, Row, Space},
    Center, Element, Fill,
};

const TOGGLE_ON_COLOR: iced::Color = iced::Color {
    r: 0.6,
    g: 0.3,
    b: 0.9,
    a: 1.0,
};

const MUTE_COLOR: iced::Color = iced::Color {
    r: 0.9,
    g: 0.2,
    b: 0.2,
    a: 1.0,
};

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
        .text_size(10)
        .spacing(3)
        .into()
}

pub fn build_mute_btn(fx_muted: bool) -> Element<'static, InterfaceMessage> {
    let label = if fx_muted { "FX MUTED" } else { "FX MUTE" };
    let btn = button(text(label).size(10).center())
        .on_press(InterfaceMessage::FxMute)
        .padding([5, 4])
        .width(Fill);
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

// 2×8 grid: 4-wide doesn't fit inside a mixer bus strip
pub fn build_preset_grid(fx: &FxSlot) -> Element<'_, InterfaceMessage> {
    let fx_id = fx.id;
    let current = fx.preset as usize;
    let presets = fx.presets();

    let mut rows: Vec<Element<InterfaceMessage>> = Vec::new();

    for row_idx in 0..8 {
        let mut buttons: Vec<Element<InterfaceMessage>> = Vec::new();
        for col in 0..2 {
            let idx = row_idx * 2 + col;
            let preset = &presets[idx];
            let is_selected = idx == current;

            let label = column![text(preset.name).size(9)].align_x(Center);

            let btn = if is_selected {
                button(label)
                    .padding([4, 2])
                    .width(Fill)
                    .style(button::primary)
            } else {
                button(label)
                    .on_press(InterfaceMessage::FxPreset(fx_id, idx as u8))
                    .padding([4, 2])
                    .width(Fill)
                    .style(button::secondary)
            };

            buttons.push(btn.into());
        }
        rows.push(Row::with_children(buttons).spacing(3).width(Fill).into());
    }

    let mut grid = Column::new().spacing(3).width(Fill);
    for r in rows {
        grid = grid.push(r);
    }

    column![
        sync_label("Preset", 10.0, fx.preset_synced),
        Space::new().height(2),
        grid,
    ]
    .width(Fill)
    .into()
}

pub fn build_param1_slider(fx: &FxSlot) -> Element<'_, InterfaceMessage> {
    let fx_id = fx.id;
    let info = fx.preset_info();
    column![
        sync_label(info.param1_label, 10.0, fx.param1_synced),
        tooltip(
            slider(0..=100, fx.param1, move |v| InterfaceMessage::FxParam1(fx_id, v))
                .width(Fill)
                // model updates per drag frame; send the settled value once on release
                .on_release(InterfaceMessage::FxParam1Send(fx_id)),
            container(text(format!("{}%", fx.param1)).size(9))
                .padding(4)
                .style(container::rounded_box),
            tooltip::Position::Top,
        )
        .gap(2),
    ]
    .width(Fill)
    .spacing(2)
    .into()
}

pub fn build_param2_toggle<'a>(
    fx_id: u8,
    off_label: &'a str,
    on_label: &'a str,
    is_on: bool,
    is_synced: bool,
) -> Element<'a, InterfaceMessage> {
    let off_btn = if !is_on {
        button(text(off_label).size(9).center())
            .padding([5, 2])
            .width(Fill)
            .style(move |theme, status| {
                let mut style = button::primary(theme, status);
                style.background = Some(iced::Background::Color(TOGGLE_ON_COLOR));
                style
            })
    } else {
        button(text(off_label).size(9).center())
            .on_press(InterfaceMessage::FxParam2(fx_id, 0))
            .padding([5, 2])
            .width(Fill)
            .style(button::secondary)
    };

    let on_btn = if is_on {
        button(text(on_label).size(9).center())
            .padding([5, 2])
            .width(Fill)
            .style(move |theme, status| {
                let mut style = button::primary(theme, status);
                style.background = Some(iced::Background::Color(TOGGLE_ON_COLOR));
                style
            })
    } else {
        button(text(on_label).size(9).center())
            .on_press(InterfaceMessage::FxParam2(fx_id, 100))
            .padding([5, 2])
            .width(Fill)
            .style(button::secondary)
    };

    let sync_dot = if is_synced {
        text("")
    } else {
        text("\u{25CF} ").size(9).color(UNSYNCED_COLOR)
    };

    row![sync_dot, off_btn, on_btn]
        .spacing(3)
        .align_y(Center)
        .width(Fill)
        .into()
}

pub fn build_tap_tempo_btn(bpm: Option<u16>) -> Element<'static, InterfaceMessage> {
    let label = match bpm {
        Some(b) => format!("TAP {} BPM", b),
        None => "TAP".to_string(),
    };
    let tap_btn = button(text(label).size(10).center())
        .on_press(InterfaceMessage::TapTempo)
        .padding([5, 4])
        .width(Fill)
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
