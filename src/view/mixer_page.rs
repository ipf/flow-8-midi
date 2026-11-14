use crate::model::{
    channels::{Bus, BusType, Channel, PhantomPowerType},
    flow8::{FLOW8Controller, RenameTarget},
    message::InterfaceMessage,
};
use crate::view::fx_widgets::{
    build_mute_btn, build_param1_knob, build_param2_pick, build_preset_pick,
    build_route_checkbox, build_tap_tempo_btn,
};
use crate::view::widgets::{
    knob, sync_dot, sync_label, v_slider, on_right_click, LEVEL_ZERO_DB,
    format_gain, format_level, format_pan, format_comp, format_lowcut, format_limiter,
    format_eq, format_send,
};
use iced::{
    widget::{
        button, column, container, row, text, text_input, tooltip,
        tooltip::Position, Column, Space,
    },
    Center, Color, Element, Fill, Length,
};
use std::time::Instant;

const FADER_HEIGHT: f32 = 150.0;
const BUS_FADER_HEIGHT: f32 = 150.0;
const METER_WIDTH: f32 = 9.0;
const KNOB_SIZE: f32 = 28.0;
// label (12) + spacing (1) + knob (28): reserve slots match real knob rows
const KNOB_SLOT_HEIGHT: f32 = 41.0;
const EQ_KNOB_SIZE: f32 = 28.0;

/// thin vertical VU bar, filled bottom-up; value is linear 0..1.
/// Always visible: dark unlit track + colored lit portion.
fn meter_bar(value: f32, gr: u8) -> Element<'static, InterfaceMessage> {
    let value = value.clamp(0.0, 1.0);
    let filled = (value * FADER_HEIGHT).max(if value > 0.0 { 2.0 } else { 0.0 });
    let color = if value > 0.95 {
        Color::from_rgb(0.9, 0.2, 0.2)
    } else if value > 0.75 {
        Color::from_rgb(0.95, 0.75, 0.1)
    } else {
        Color::from_rgb(0.2, 0.8, 0.3)
    };
    let track_style = |_: &iced::Theme| container::Style {
        background: Some(Color::from_rgb(0.09, 0.09, 0.12).into()),
        ..Default::default()
    };
    let mut col = Column::new().height(Length::Fixed(FADER_HEIGHT));
    // unlit upper part
    let top = FADER_HEIGHT - filled;
    // gain reduction: up to 3 red 4 px segments hanging from the meter top
    // (0x22 GR bits, 0..=3) — only visible while the compressor/limiter is
    // actually reducing
    let gr_h = (gr as f32 * 4.0).min(top.max(0.0));
    if gr_h > 0.5 {
        col = col.push(
            container(Space::new())
                .width(Length::Fixed(METER_WIDTH))
                .height(Length::Fixed(gr_h))
                .style(move |_| container::Style {
                    background: Some(Color::from_rgb(0.9, 0.15, 0.15).into()),
                    ..Default::default()
                }),
        );
    }
    let top = top - gr_h;
    if top > 0.5 {
        col = col.push(
            container(Space::new())
                .width(Length::Fixed(METER_WIDTH))
                .height(Length::Fixed(top))
                .style(track_style),
        );
    }
    // lit lower part
    if filled > 0.5 {
        col = col.push(
            container(Space::new())
                .width(Length::Fixed(METER_WIDTH))
                .height(Length::Fixed(filled))
                .style(move |_| container::Style {
                    background: Some(color.into()),
                    ..Default::default()
                }),
        );
    }
    col.width(Length::Fixed(METER_WIDTH)).into()
}

/// clip light: red dot above the fader, latched for 2 s after hitting 0 dBFS.
/// Fixed-height slot — both states occupy the same space so the fader block
/// never shifts the layout.
fn clip_dot(clip_time: Option<Instant>) -> Element<'static, InterfaceMessage> {
    let active = clip_time
        .map(|t| t.elapsed().as_millis() < 2000)
        .unwrap_or(false);
    container(if active {
        text("\u{25CF}")
            .size(10)
            .color(Color::from_rgb(0.95, 0.15, 0.15))
    } else {
        text("").size(10)
    })
    .height(Length::Fixed(12.0))
    .align_y(Center)
    .into()
}

pub fn view_mixer(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    let channels_row: Vec<Element<InterfaceMessage>> = controller
        .channels
        .iter()
        .map(|c| build_channel_strip(c, controller))
        .collect();

    let channels_section = container(
        iced::widget::Row::with_children(channels_row)
            .spacing(3)
            .width(Fill)
            .height(Fill),
    )
    .width(Length::FillPortion(6))
    .height(Fill);

    // Main fader rightmost: monitors, then FX returns (FX1/FX2 = the send
    // targets' return level — without these the FX send knobs are silent),
    // then Main
    let mut buses: Vec<&Bus> = controller.buses.iter().collect();
    buses.sort_by_key(|b| match b.bus_type {
        BusType::Monitor => 0,
        BusType::Fx => 1,
        BusType::Main => 2,
    });
    // FX strips are normal width now: M + dropdown + knob + toggle fit
    let mut bus_row = iced::widget::Row::new().spacing(2).width(Fill).height(Fill);
    let mut prev_type: Option<BusType> = None;
    for b in buses {
        // same visual break between the bus groups (Mon | FX | Main) as
        // between the channels and the buses
        if prev_type.is_some_and(|pt| pt != b.bus_type) {
            bus_row = bus_row.push(Space::new().width(3));
        }
        prev_type = Some(b.bus_type);
        bus_row = bus_row.push(
            container(build_bus_strip(b, controller))
                .width(Length::FillPortion(1))
                .height(Fill),
        );
    }

    let buses_section = container(bus_row)
        .width(Length::FillPortion(4))
        .height(Fill);

    row![channels_section, Space::new().width(3), buses_section]
        .width(Fill)
        .height(Fill)
        .padding([0, 10])
        .into()
}

const NAME_SLOT_HEIGHT: f32 = 16.0;
const PHANTOM_SLOT_HEIGHT: f32 = 30.0;
/// header slot heights shared by channel AND bus strips: label line,
/// rename-button slot, M/S row slot — keeps the knob sections starting
/// at the same Y on every strip
const LABEL_SLOT_HEIGHT: f32 = 18.0;
const MUTE_SLOT_HEIGHT: f32 = 24.0;

fn build_channel_strip<'a>(
    channel: &'a Channel,
    controller: &'a FLOW8Controller,
) -> Element<'a, InterfaceMessage> {
    let mut col = Column::new()
        .width(Fill)
        .height(Fill)
        .align_x(Center)
        .spacing(4)
        .padding([8, 6]);

    let renaming = controller.rename_target == Some(RenameTarget::Channel(channel.id));
    if renaming {
        // inline rename editor (Enter = submit, ✕ = cancel)
        col = col.push(
            column![
                text_input("name", &controller.rename_buffer)
                    .on_input(InterfaceMessage::RenameChanged)
                    .on_submit(InterfaceMessage::RenameSubmit)
                    .size(12)
                    .width(Fill),
                button(text("\u{2715}").size(10))
                    .on_press(InterfaceMessage::RenameCancel)
                    .padding([2, 6])
                    .style(button::secondary),
            ]
            .align_x(Center)
            .spacing(4),
        );
    } else {
        col = col.push(
            container(text(channel.display_label()).size(13))
                .height(Length::Fixed(LABEL_SLOT_HEIGHT))
                .align_y(Center),
        );

        // name (or ✎ when empty) doubles as the rename button
        let name_label = if channel.name.is_empty() {
            "\u{270E}".to_string()
        } else {
            channel.name.clone()
        };
        col = col.push(
            container(
                button(text(name_label).size(13))
                    .on_press(InterfaceMessage::RenameStartChannel(channel.id))
                    .padding([1, 4])
                    .style(button::text),
            )
            .height(Length::Fixed(NAME_SLOT_HEIGHT))
            .align_y(Center),
        );
    }

    col = add_mute_solo(col, channel);

    col = col.push(build_phantom_slot(channel));

    let strip = &channel.channel_strip;
    let ch_id = channel.id;

    // ── SSL-style strip: gain / EQ / dynamics / sends knobs above the fader ──
    if channel.has_gain() {
        col = col.push(knob_slot(
            "Gain",
            strip.gain_synced,
            0..=127,
            strip.gain,
            64,
            move |v| InterfaceMessage::Gain(ch_id, v),
            format_gain(strip.gain),
        ));
    } else {
        col = col.push(Space::new().height(Length::Fixed(KNOB_SLOT_HEIGHT)));
    }

    let eq = &channel.four_band_eq;
    let eq_id = channel.id;
    col = col.push(
        row![
            knob_slot("Low", eq.low_synced, 0..=127, eq.low, 64, move |v| {
                InterfaceMessage::EqLow(eq_id, v)
            }, format_eq(eq.low)),
            knob_slot("LoMid", eq.low_mid_synced, 0..=127, eq.low_mid, 64, move |v| {
                InterfaceMessage::EqLowMid(eq_id, v)
            }, format_eq(eq.low_mid)),
        ]
        .spacing(4),
    );
    col = col.push(
        row![
            knob_slot("HiMid", eq.hi_mid_synced, 0..=127, eq.hi_mid, 64, move |v| {
                InterfaceMessage::EqHiMid(eq_id, v)
            }, format_eq(eq.hi_mid)),
            knob_slot("High", eq.hi_synced, 0..=127, eq.hi, 64, move |v| {
                InterfaceMessage::EqHi(eq_id, v)
            }, format_eq(eq.hi)),
        ]
        .spacing(4),
    );

    if channel.has_low_cut() {
        col = col.push(
            row![
                knob_slot("Cut", strip.low_cut_synced, 0..=127, strip.low_cut, 0, move |v| {
                    InterfaceMessage::LowCut(ch_id, v)
                }, format_lowcut(strip.low_cut)),
                knob_slot("Comp", strip.compressor_synced, 0..=100, strip.compressor, 0, move |v| {
                    InterfaceMessage::Compressor(ch_id, v)
                }, format_comp(strip.compressor)),
            ]
            .spacing(4),
        );
    } else {
        col = col.push(Space::new().height(Length::Fixed(KNOB_SLOT_HEIGHT)));
    }

    let sends = &channel.sends;
    let send_id = channel.id;
    col = col.push(
        row![
            knob_slot("Mon1", sends.mon1_synced, 0..=127, sends.mon1, 0, move |v| {
                InterfaceMessage::SendMon1(send_id, v)
            }, format_send(sends.mon1)),
            knob_slot("Mon2", sends.mon2_synced, 0..=127, sends.mon2, 0, move |v| {
                InterfaceMessage::SendMon2(send_id, v)
            }, format_send(sends.mon2)),
        ]
        .spacing(4),
    );
    col = col.push(
        row![
            knob_slot("FX1", sends.fx1_synced, 0..=127, sends.fx1, 0, move |v| {
                InterfaceMessage::SendFx1(send_id, v)
            }, format_send(sends.fx1)),
            knob_slot("FX2", sends.fx2_synced, 0..=127, sends.fx2, 0, move |v| {
                InterfaceMessage::SendFx2(send_id, v)
            }, format_send(sends.fx2)),
        ]
        .spacing(4),
    );

    // bottom-anchor: faders of all strips line up; the Pan slot sits
    // directly above the fader block so it lines up with the buses'
    // Pan/Limiter band on every strip
    col = col.push(Space::new().height(Fill));

    col = col.push(knob_slot(
        "Pan",
        strip.balance_synced,
        0..=127,
        strip.balance,
        64,
        move |v| InterfaceMessage::Balance(ch_id, v),
        format_pan(strip.balance),
    ));

    col = col.push(
        column![
            sync_label("Level", 12.0, strip.level_synced),
            clip_dot(channel.clip_time),
            row![
                on_right_click(
                    v_slider(1..=127, strip.level, FADER_HEIGHT, move |v| {
                        InterfaceMessage::Level(ch_id, v)
                    }, format_level(strip.level)),
                    InterfaceMessage::Level(ch_id, LEVEL_ZERO_DB),
                ),
                meter_bar(channel.meter, channel.gr),
            ]
            .align_y(iced::alignment::Vertical::Bottom)
            .spacing(2),
            text(format_level(strip.level)).size(10),
        ]
        .align_x(Center)
        .spacing(2),
    );

    container(col)
        .style(container::rounded_box)
        .width(Fill)
        .height(Fill)
        .padding(2)
        .into()
}

fn knob_slot<'a, F>(
    label: &'a str,
    is_synced: bool,
    range: std::ops::RangeInclusive<u8>,
    value: u8,
    default: u8,
    on_change: F,
    tip: String,
) -> Element<'a, InterfaceMessage>
where
    F: Fn(u8) -> InterfaceMessage + 'a,
{
    knob_slot_s(label, is_synced, range, value, default, KNOB_SIZE, on_change, tip)
}

fn knob_slot_s<'a, F>(
    label: &'a str,
    is_synced: bool,
    range: std::ops::RangeInclusive<u8>,
    value: u8,
    default: u8,
    size: f32,
    on_change: F,
    tip: String,
) -> Element<'a, InterfaceMessage>
where
    F: Fn(u8) -> InterfaceMessage + 'a,
{
    column![
        sync_label(label, 9.0, is_synced),
        knob(range, value, default, size, on_change, tip),
    ]
    .align_x(Center)
    .spacing(1)
    .into()
}

fn build_phantom_slot(channel: &Channel) -> Element<'_, InterfaceMessage> {
    let content: Element<'_, InterfaceMessage> = if let PhantomPowerType::Set48v =
        channel.phantom_pwr.phantom_power_type
    {
        let awaiting_confirm = channel
            .phantom_last_click
            .map(|t| t.elapsed().as_millis() < 500)
            .unwrap_or(false);

        let base = button(if awaiting_confirm {
            text("\u{26A0} 48V \u{26A0}").size(11)
        } else if channel.phantom_pwr.is_on {
            text("48V ON").size(11)
        } else {
            text("48V").size(11)
        })
        .on_press(InterfaceMessage::PhantomPower(channel.id))
        .padding([3, 10]);

        let btn = if awaiting_confirm {
            base.style(button::primary)
        } else if channel.phantom_pwr.is_on {
            base.style(button::danger)
        } else {
            base.style(button::secondary)
        };

        let tip_text = if awaiting_confirm {
            "\u{26A0} CLICK AGAIN to confirm!\nPhantom Power can damage unbalanced microphones."
        } else {
            "\u{26A0} Double-click to toggle Phantom Power (+48V)\nWARNING: May damage unbalanced mics!"
        };

        let tip = tooltip(
            btn,
            container(text(tip_text).size(12))
                .padding(8)
                .style(container::rounded_box),
            Position::FollowCursor,
        )
        .gap(5);

        row![sync_dot(channel.phantom_synced), tip]
            .align_y(Center)
            .spacing(2)
            .into()
    } else {
        button(text(" ").size(11))
            .padding([3, 10])
            .style(button::text)
            .into()
    };
    container(content)
        .height(Length::Fixed(PHANTOM_SLOT_HEIGHT))
        .align_y(Center)
        .into()
}

fn add_mute_solo<'a>(
    column: Column<'a, InterfaceMessage>,
    channel: &'a Channel,
) -> Column<'a, InterfaceMessage> {
    let mute_btn = {
        let btn = button(text("M").size(12))
            .on_press(InterfaceMessage::Mute(channel.id, channel.is_muted))
            .padding([4, 10]);
        if channel.is_muted {
            btn.style(button::danger)
        } else {
            btn.style(button::secondary)
        }
    };

    let solo_btn = {
        let btn = button(text("S").size(12))
            .on_press(InterfaceMessage::Solo(channel.id, channel.is_soloed))
            .padding([4, 10]);
        if channel.is_soloed {
            btn.style(button::success)
        } else {
            btn.style(button::secondary)
        }
    };

    column.push(
        container(
            row![
                sync_dot(channel.mute_synced),
                mute_btn,
                sync_dot(channel.solo_synced),
                solo_btn,
            ]
            .align_y(Center)
            .spacing(2),
        )
        .height(Length::Fixed(MUTE_SLOT_HEIGHT))
        .align_y(Center),
    )
}

fn build_bus_strip<'a>(
    bus: &'a Bus,
    controller: &'a FLOW8Controller,
) -> Element<'a, InterfaceMessage> {
    let mut col = Column::new()
        .width(Fill)
        .height(Fill)
        .align_x(Center)
        .spacing(4)
        .padding([8, 3]);

    let bus_idx = bus.index;
    let bus_id = bus.id;

    // header slots mirror the channel strips (label / rename button /
    // M-S row / phantom) so the knob sections start at the same Y on
    // every strip
    col = col.push(
        container(text(bus.label()).size(13))
            .height(Length::Fixed(LABEL_SLOT_HEIGHT))
            .align_y(Center),
    );
    col = col.push(Space::new().height(Length::Fixed(NAME_SLOT_HEIGHT)));
    if bus.bus_type == BusType::Fx && bus.index == 3 {
        // FX1: M in the channels' M/S band (same button look)
        col = col.push(
            container(build_mute_btn(controller.fx_muted))
                .height(Length::Fixed(MUTE_SLOT_HEIGHT))
                .align_y(Center),
        );
    } else {
        col = col.push(Space::new().height(Length::Fixed(MUTE_SLOT_HEIGHT)));
    }
    col = col.push(Space::new().height(Length::Fixed(PHANTOM_SLOT_HEIGHT)));

    // 9-band EQ knobs (2 per row), same data as the EQ page. FX buses
    // have no 9-band EQ — their strip hosts the FX controls instead
    // (preset dropdown, param1 knob, param2 toggle, routing; the old Fx
    // page is gone).
    if bus.bus_type == BusType::Fx {
        let fx_idx = (bus.index - 3) as usize; // buses: 0=Main 1=Mon1 2=Mon2 3=FX1 4=FX2
        if let Some(f) = controller.fx_slots.get(fx_idx) {
            let info = f.preset_info();
            col = col.push(build_preset_pick(f));
            col = col.push(build_param1_knob(f));
            col = col.push(build_param2_pick(
                f.id,
                info.param2_off,
                info.param2_on,
                f.param2_is_on(),
                f.param2_synced,
            ));
            if fx_idx == 0 {
                // TAP sits between the param2 dropdown and the routing
                col = col.push(build_tap_tempo_btn(controller.current_tap_bpm()));
            }
            col = col.push(
                // routing stacked vertically: reads cleaner than a
                // horizontal row in a narrow strip; the unsynced dot
                // shows until an FXSetup (any param/routing touch) put
                // the routing on the device
                column![
                    row![sync_dot(f.route_synced), build_route_checkbox(f, 0, "Main")],
                    row![sync_dot(f.route_synced), build_route_checkbox(f, 1, "Mon1")],
                    row![sync_dot(f.route_synced), build_route_checkbox(f, 2, "Mon2")],
                ]
                .spacing(2),
            );
        }
    } else if bus.has_nine_band_eq() {
        let eq = &bus.nine_band_eq;
        let bands: [(&str, u8, u8); 9] = [
            ("62", eq.freq_62_hz, 0),
            ("125", eq.freq_125_hz, 1),
            ("250", eq.freq_250_hz, 2),
            ("500", eq.freq_500_hz, 3),
            ("1k", eq.freq_1_khz, 4),
            ("2k", eq.freq_2_khz, 5),
            ("4k", eq.freq_4_khz, 6),
            ("8k", eq.freq_8_khz, 7),
            ("16k", eq.freq_16_khz, 8),
        ];
        // 2 knobs per row: 3×3 overflows the strip width
        for chunk in bands.chunks(2) {
            let mut eq_row = row![].spacing(2);
            for &(label, value, band_index) in chunk {
                eq_row = eq_row.push(knob_slot_s(
                    label,
                    eq.bands_synced[band_index as usize],
                    0..=127,
                    value,
                    64,
                    EQ_KNOB_SIZE,
                    move |v| InterfaceMessage::BusNineBandEq(bus_idx, bus_id, band_index, v),
                    format_eq(value),
                ));
            }
            col = col.push(eq_row);
        }
    }

    // bottom-anchor: bus faders line up with the channel faders
    col = col.push(Space::new().height(Fill));

    // pan/limiter band, one row on every strip: Main shares Pan +
    // Limiter, the monitors keep Limiter — same slot as the channels'
    // Pan so all bands align across the mixer. FX strips skip it.
    if bus.bus_type != BusType::Fx {
        if bus.has_balance() {
            let mut band = row![knob_slot(
                "Pan",
                bus.bus_strip.balance_synced,
                0..=127,
                bus.bus_strip.balance,
                64,
                move |v| InterfaceMessage::BusBalance(bus_idx, bus_id, v),
                format_pan(bus.bus_strip.balance),
            )]
            .spacing(4);
            if bus.has_limiter() {
                band = band.push(knob_slot(
                    "Limiter",
                    bus.bus_strip.limiter_synced,
                    0..=127,
                    bus.bus_strip.limiter,
                    127,
                    move |v| InterfaceMessage::BusLimiter(bus_idx, bus_id, v),
                    format_limiter(bus.bus_strip.limiter),
                ));
            }
            col = col.push(band);
        } else if bus.has_limiter() {
            col = col.push(knob_slot(
                "Limiter",
                bus.bus_strip.limiter_synced,
                0..=127,
                bus.bus_strip.limiter,
                127,
                move |v| InterfaceMessage::BusLimiter(bus_idx, bus_id, v),
                format_limiter(bus.bus_strip.limiter),
            ));
        }
    }

    col = col.push(
        column![
            sync_label("Level", 12.0, bus.bus_strip.level_synced),
            // mirrors the channel strips' clip light slot so Level labels align
            Space::new().height(Length::Fixed(12.0)),
            row![
                on_right_click(
                    v_slider(1..=127, bus.bus_strip.level, BUS_FADER_HEIGHT, move |v| {
                        InterfaceMessage::BusLevel(bus_idx, bus_id, v)
                    }, format_level(bus.bus_strip.level)),
                    InterfaceMessage::BusLevel(bus_idx, bus_id, LEVEL_ZERO_DB),
                ),
                meter_bar(bus.meter, bus.gr),
            ]
            .align_y(iced::alignment::Vertical::Bottom)
            .spacing(2),
            text(format_level(bus.bus_strip.level)).size(10),
        ]
        .align_x(Center)
        .spacing(2),
    );

    container(col)
        .style(container::rounded_box)
        .width(Fill)
        .height(Fill)
        .padding(2)
        .into()
}
