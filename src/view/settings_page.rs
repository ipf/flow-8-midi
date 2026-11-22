// Settings page: device + app settings only (the About info, manual and
// debug log live on the About page now).
use crate::model::{
    flow8::{FLOW8Controller, SyncInterval},
    message::InterfaceMessage,
};
use crate::view::snapshots_page::snapshots_section;
use iced::{
    widget::{button, column, container, pick_list, row, scrollable, text, tooltip, Space},
    Center, Element, Fill, Length, Theme,
};

pub fn view_settings(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    let title = text("Settings").size(18);

    let appearance_section = build_appearance_section(controller);
    let device_section = build_device_section(controller);

    // device settings card on the left; snapshots + UI settings stacked on
    // the right
    let right_column = column![
        snapshots_section(controller),
        Space::new().height(12),
        appearance_section,
    ]
    .width(Fill);

    let content = column![
        title,
        Space::new().height(12),
        row![
            container(device_section)
                .width(Length::FillPortion(1)),
            Space::new().width(8),
            container(right_column)
                .width(Length::FillPortion(1)),
        ]
        .width(Fill),
        Space::new().height(12),
    ]
    .width(Fill)
    .padding([0, 10]);

    scrollable(content).height(Fill).into()
}

// ── FLOW 8 device settings (BLE op 0x25, ids from the official app) ──

fn setting_row<'a>(
    label: &'a str,
    tip: &'a str,
    id: u8,
    options: &[(&'a str, u8)],
    current: Option<u8>,
) -> Element<'a, InterfaceMessage> {
    let mut buttons = row![].spacing(4);
    for &(name, value) in options {
        let b = button(text(name).size(11))
            .on_press(InterfaceMessage::SetDeviceSetting(id, value))
            .padding([3, 8]);
        // active option: bright filled style — clearly distinct from the
        // grey secondary buttons
        buttons = buttons.push(if current == Some(value) {
            b.style(|_theme, _status| button::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    0.15, 0.45, 0.9,
                ))),
                text_color: iced::Color::WHITE,
                border: iced::Border::default(),
                ..button::Style::default()
            })
            .padding([3, 8])
        } else {
            b.style(button::secondary).padding([3, 8])
        });
    }
    let content = tooltip(
        column![text(label).size(12), Space::new().height(6), buttons].width(Fill),
        container(text(tip).size(10))
            .padding(6)
            .style(container::rounded_box),
        tooltip::Position::FollowCursor,
    )
    .gap(4);

    container(content)
        .width(Fill)
        .padding([10, 12])
        .style(container::rounded_box)
        .into()
}

fn build_device_section(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    let cur = |id: u8| controller.ble_settings[id as usize];

    // two cards per row, related settings side by side
    let mut cards = vec![
        setting_row("Footswitch", "What the connected footswitch does:\nFX mute = tap mutes the FX bus\nSnapshot = tap recalls the next snapshot", 0x03, &[("FX mute", 0), ("Recall snapshot", 1)], cur(0x03)),
        setting_row("BT/USB playback", "Where the BT/USB player plays:\nMain mix = also on the MAIN outputs\nPhones = only on the phones out", 0x02, &[("Main mix", 0), ("Phones", 1)], cur(0x02)),
        setting_row("Headphones source", "What the headphones play:\nMain = the MAIN bus mix\nMon 1/2 = the MONITOR 1/2 bus mix", 0x0c, &[("Main", 0), ("Mon 1/2", 1)], cur(0x0c)),
        setting_row("Headphones tap point", "Only affects the Mon 1/2 phones mix:\nPre = before the Mon 1/2 fader\nPost = after the Mon 1/2 fader", 0x0d, &[("Pre", 0), ("Post", 1)], cur(0x0d)),
        setting_row("Recording / Streaming", "USB return mode:\nAufnahme = multichannel, pre-fader\nStreaming = stereo, post-fader\n(plays on Ch 5/6 and Ch 7/8)", 0x07, &[("Aufnahme", 0), ("Streaming", 1)], cur(0x07)),
        setting_row("Ch 5/6 · 7/8 audio", "Which audio the BT/USB channel plays:\nBT audio = Bluetooth input\nUSB player = USB return playback", 0x01, &[("BT audio", 0), ("USB player", 1)], cur(0x01)),
        setting_row("Mon out 1/2", "What the MONITOR 1/2 outputs carry:\nMon buses = their own mix\nUSB 1/2 / USB 3/4 = computer audio", 0x08, &[("Mon buses", 0), ("USB 1/2", 1), ("USB 3/4", 2)], cur(0x08)),
        setting_row("Mon 1/2 source", "Where the MONITOR 1/2 outputs tap:\nPre = before the Mon 1/2 fader\nPost = after the Mon 1/2 fader", 0x11, &[("Pre", 0), ("Post", 1)], cur(0x11)),
        setting_row("Stereo link", "Links MONITOR 1 and 2 into one\nstereo output (1 = L, 2 = R).", 0x0b, &[("Off", 0), ("Linked", 1)], cur(0x0b)),
        setting_row("USB 1/2 to channel", "What channel strip 5/6 gets:\nLINE = the physical line inputs\nUSB = computer USB return ch 1/2", 0x09, &[("LINE", 0), ("USB", 1)], cur(0x09)),
        setting_row("USB 3/4 to channel", "What channel strip 7/8 gets:\nLINE = the physical line inputs\nUSB = computer USB return ch 3/4", 0x0a, &[("LINE", 0), ("USB", 1)], cur(0x0a)),
        setting_row("Main pad", "Attenuates the MAIN XLR outputs\nby 10 dBV (for sensitive inputs).", 0x0e, &[("Off", 0), ("-10 dBV", 1)], cur(0x0e)),
        setting_row("Monitor pad", "Attenuates the MONITOR outputs\nby 10 dBV (for sensitive inputs).", 0x0f, &[("Off", 0), ("-10 dBV", 1)], cur(0x0f)),
    ];
    let mut grid = iced::widget::Column::new().spacing(8);
    while !cards.is_empty() {
        let pair: Vec<_> = cards.drain(..cards.len().min(2)).collect();
        grid = grid.push(iced::widget::Row::with_children(pair).spacing(8));
    }

    container(
        column![
            text("FLOW 8 Device").size(14),
            Space::new().height(8),
            grid,
            Space::new().height(6),
            text("Click sends the setting over BLE. Blue = active device setting (read at connect).")
                .size(10),
        ]
        .padding([12, 14])
        .width(Fill),
    )
    .style(container::rounded_box)
    .width(Fill)
    .into()
}

fn build_appearance_section(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    let header = text("Appearance & Sync").size(14);

    let theme_row = row![
        text("Theme").size(12),
        Space::new().width(12),
        pick_list(Theme::ALL, Some(&controller.theme), InterfaceMessage::ThemeChanged)
            .text_size(12)
            .padding([4, 8]),
    ]
    .align_y(Center)
    .spacing(8);

    let sync_row = row![
        text("Sync interval").size(12),
        Space::new().width(12),
        pick_list(
            SyncInterval::ALL,
            Some(controller.sync_interval),
            InterfaceMessage::SyncIntervalChanged,
        )
        .text_size(12)
        .padding([4, 8]),
    ]
    .align_y(Center)
    .spacing(8);

    container(
        column![
            header,
            Space::new().height(8),
            theme_row,
            Space::new().height(6),
            sync_row,
        ]
        .padding([12, 14])
        .width(Fill),
    )
    .style(container::rounded_box)
    .width(Fill)
    .into()
}
