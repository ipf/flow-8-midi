use crate::model::{
    channels::Bus,
    flow8::FLOW8Controller,
    message::InterfaceMessage,
    page::Page,
};
use crate::view::widgets::{
    h_slider, format_level, format_limiter, format_send, sync_dot, sync_label, v_slider,
};
use iced::{
    widget::{button, column, container, row, text, Column, Row, Space},
    Center, Element, Fill,
};

const SEND_FADER_HEIGHT: f32 = 280.0;
const MASTER_FADER_HEIGHT: f32 = 280.0;
const MASTER_WIDTH: f32 = 110.0;

// (tab label, tab index). bus index = tab + 1 in controller.buses
// (0 = Main, 1 = Mon 1, 2 = Mon 2, 3 = FX 1, 4 = FX 2)
const TABS: [(&str, usize); 4] = [("Mon 1", 0), ("Mon 2", 1), ("FX 1", 2), ("FX 2", 3)];

pub fn view_sends(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    let active = controller.monitor_tab.min(TABS.len() - 1);

    // ── tab bar (like the official app's bus selector) ──
    let mut tab_bar = Row::new().spacing(6).align_y(Center);
    for (label, idx) in TABS {
        let btn = if idx == active {
            button(text(label).size(13).center()).padding([5, 16])
        } else {
            button(text(label).size(13).center())
                .on_press(InterfaceMessage::MonitorTab(idx))
                .padding([5, 16])
                .style(button::secondary)
        };
        tab_bar = tab_bar.push(btn);
    }
    let tab_bar = container(tab_bar).center_x(Fill).padding(4);

    // ── bus for the selected tab ──
    let bus_index = active + 1;
    let bus = controller.buses.iter().find(|b| b.index == bus_index as u8);
    let default_bus = Bus::default(); // fallback should never trigger (buses 0..=4 exist)
    let bus = bus.unwrap_or(&default_bus);

    // ── master strip (level fader + limiter + EQ link) ──
    let bus_idx = bus.index;
    let bus_id = bus.id;
    let master_label = match bus.bus_type {
        crate::model::channels::BusType::Fx => format!("FX {} return", bus.index - 2),
        _ => format!("{} master", bus.label()),
    };

    let mut master_col = Column::new()
        .spacing(6)
        .align_x(Center)
        .padding([10, 6]);

    master_col = master_col.push(text(master_label).size(12));
    master_col = master_col.push(
        column![
            sync_label("Level", 12.0, bus.bus_strip.level_synced),
            v_slider(1..=127, bus.bus_strip.level, MASTER_FADER_HEIGHT, move |v| {
                InterfaceMessage::BusLevel(bus_idx, bus_id, v)
            }, format_level(bus.bus_strip.level)),
            text(format_level(bus.bus_strip.level)).size(10),
        ]
        .align_x(Center)
        .spacing(4),
    );

    if bus.has_limiter() {
        master_col = master_col.push(
            column![
                sync_label("Limiter", 12.0, bus.bus_strip.limiter_synced),
                h_slider(0..=127, bus.bus_strip.limiter, move |v| {
                    InterfaceMessage::BusLimiter(bus_idx, bus_id, v)
                }, format_limiter(bus.bus_strip.limiter)),
            ]
            .align_x(Center)
            .spacing(2)
            .width(Fill)
            .padding([0, 2]),
        );
    }

    if bus.has_nine_band_eq() || bus.bus_type == crate::model::channels::BusType::Fx {
        master_col = master_col.push(
            button(text("9-Band EQ \u{2192}").size(11).center())
                .on_press(InterfaceMessage::NavigateTo(Page::Eq))
                .padding([5, 8])
                .style(button::secondary),
        );
    }

    let master_strip = container(master_col)
        .padding(4)
        .width(iced::Length::Fixed(MASTER_WIDTH));

    // ── one send fader per channel, for the selected bus ──
    let active_static = active;
    let send_children: Vec<Element<InterfaceMessage>> = controller
        .channels
        .iter()
        .map(|c| build_channel_send_fader(c, active_static))
        .collect();

    let sends_row = Row::with_children(send_children)
        .spacing(4)
        .width(Fill)
        .height(Fill);

    let content = column![
        tab_bar,
        row![
            master_strip,
            Space::new().width(6),
            sends_row,
        ]
        .width(Fill)
        .height(Fill),
    ]
    .width(Fill)
    .height(Fill)
    .padding([0, 10]);

    content.into()
}

fn build_channel_send_fader(channel: &crate::model::channels::Channel, tab: usize) -> Element<'_, InterfaceMessage> {
    let ch_id = channel.id;
    let sends = &channel.sends;

    let (value, synced, on_change): (u8, bool, Box<dyn Fn(u8) -> InterfaceMessage + '_>) = match tab {
        0 => (sends.mon1, sends.mon1_synced, Box::new(move |v| InterfaceMessage::SendMon1(ch_id, v))),
        1 => (sends.mon2, sends.mon2_synced, Box::new(move |v| InterfaceMessage::SendMon2(ch_id, v))),
        2 => (sends.fx1, sends.fx1_synced, Box::new(move |v| InterfaceMessage::SendFx1(ch_id, v))),
        _ => (sends.fx2, sends.fx2_synced, Box::new(move |v| InterfaceMessage::SendFx2(ch_id, v))),
    };

    let mut header: Column<'_, InterfaceMessage> = column![text(channel.display_label()).size(13)]
        .align_x(Center);
    if !channel.name.is_empty() {
        header = header.push(text(&channel.name).size(10));
    }

    let content = column![
        header,
        Space::new().height(8),
        sync_dot(synced),
        v_slider(0..=127, value, SEND_FADER_HEIGHT, on_change, format_send(value)),
        text(format_send(value)).size(10),
    ]
    .align_x(Center)
    .spacing(4)
    .width(Fill)
    .padding([8, 4]);

    container(content)
        .style(container::rounded_box)
        .width(Fill)
        .height(Fill)
        .into()
}
