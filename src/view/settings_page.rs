use crate::logger;
use crate::model::{
    flow8::{FLOW8Controller, SyncInterval},
    message::InterfaceMessage,
};
use crate::view::snapshots_page::snapshots_section;
#[cfg(any(debug_assertions, feature = "dev-tools"))]
use crate::service::sysex_parser;
use iced::{
    widget::{button, column, container, pick_list, row, scrollable, text, tooltip, Space},
    Center, Element, Fill, Length, Theme,
};

const VISIBLE_ENTRIES: usize = 200;
#[cfg(any(debug_assertions, feature = "dev-tools"))]
const HEX_PREVIEW_BYTES: usize = 256;

pub fn view_settings(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    let title = text("Settings").size(18);

    let appearance_section = build_appearance_section(controller);
    let device_section = build_device_section(controller);
    let manual_section = build_manual_section();
    let log_section = build_log_section(controller);

    let about_section = build_about_section();

    let mut content = column![
        title,
        Space::new().height(12),
        about_section,
        Space::new().height(16),
        device_section,
        Space::new().height(16),
        snapshots_section(controller),
        Space::new().height(16),
        appearance_section,
        Space::new().height(16),
        manual_section,
        Space::new().height(16),
        log_section,
    ]
    .width(Fill)
    .padding([0, 10]);

    #[cfg(any(debug_assertions, feature = "dev-tools"))]
    if let Some(ref dump) = controller.last_sysex_dump {
        content = content.push(Space::new().height(16));
        content = content.push(build_hex_viewer(dump));
    }

    content = content.push(Space::new().height(12));

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
        buttons = buttons.push(if current == Some(value) {
            b.style(button::primary)
        } else {
            b.style(button::secondary)
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
        setting_row("Phones only", "Mutes the MAIN outputs and plays\neverything only on the phones out.", 0x02, &[("Main + phones", 0), ("Phones only", 1)], cur(0x02)),
        setting_row("Phones mix", "What the phones output plays:\nMain mix = the MAIN bus\nMon 1/2 mix = the MONITOR 1/2 bus mix", 0x0c, &[("Main mix", 0), ("Mon 1/2 mix", 1)], cur(0x0c)),
        setting_row("Phones tap point", "Only affects the Mon 1/2 phones mix:\nPre-fader = before the Mon 1/2 fader\nPost-fader = after the Mon 1/2 fader", 0x0d, &[("Pre-fader", 0), ("Post-fader", 1)], cur(0x0d)),
        setting_row("USB streaming", "How many channels the computer\nsends back to the mixer over USB\n(plays on Ch 5/6 and Ch 7/8).", 0x07, &[("2-channel", 0), ("4-channel", 1)], cur(0x07)),
        setting_row("Ch 5/6 · 7/8 audio", "Which audio the BT/USB channel plays:\nBT audio = Bluetooth input\nUSB player = USB return playback", 0x01, &[("BT audio", 0), ("USB player", 1)], cur(0x01)),
        setting_row("Mon 1/2 source", "What the MONITOR 1/2 outputs carry:\nMonitor buses = their own mix\nUSB 1/2 / USB 3/4 = computer audio", 0x08, &[("Monitor buses", 0), ("USB 1/2", 1), ("USB 3/4", 2)], cur(0x08)),
        setting_row("Mon link", "Links MONITOR 1 and 2 into one\nstereo output (1 = L, 2 = R).", 0x0b, &[("Unlinked", 0), ("Linked", 1)], cur(0x0b)),
        setting_row("Ch 5/6 source", "What input channel strip 5/6 gets:\nLINE = the physical line inputs\nUSB = computer USB return ch 1/2", 0x09, &[("LINE", 0), ("USB", 1)], cur(0x09)),
        setting_row("Ch 7/8 source", "What input channel strip 7/8 gets:\nLINE = the physical line inputs\nUSB = computer USB return ch 3/4", 0x0a, &[("LINE", 0), ("USB", 1)], cur(0x0a)),
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
            text("Click sends the setting over BLE. Highlighted = last value sent (not read back from device).")
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

fn build_manual_section() -> Element<'static, InterfaceMessage> {
    let header = text("Manual").size(14);
    let description = text("Read the user manual for setup and usage instructions.").size(12);

    let open_btn = button(text("Open Manual").size(12))
        .on_press(InterfaceMessage::OpenManual)
        .padding([6, 14])
        .style(button::secondary);

    container(
        column![header, Space::new().height(6), description, Space::new().height(8), open_btn]
            .padding([12, 14])
            .width(Fill),
    )
    .style(container::rounded_box)
    .width(Fill)
    .into()
}

fn build_log_section(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    let header = text("Debug Log").size(14);

    let device_label = match &controller.connected_device_name {
        Some(name) => format!("Connected: {}", name),
        None => "Not connected".to_string(),
    };

    let header_row = row![
        header,
        Space::new().width(Fill),
        text(device_label).size(11),
    ]
    .align_y(Center);

    let entries = logger::get_recent_entries(VISIBLE_ENTRIES);
    let count = logger::entry_count();

    let mut log_col = iced::widget::Column::new().spacing(1);

    if count > VISIBLE_ENTRIES {
        log_col = log_col.push(
            text(format!("... ({} earlier entries omitted)", count - VISIBLE_ENTRIES)).size(10),
        );
    }

    for entry in &entries {
        let entry_text = text(format!("{}", entry)).size(11);
        let styled = match entry.level {
            logger::LogLevel::Error => entry_text.color(iced::Color::from_rgb(1.0, 0.3, 0.3)),
            logger::LogLevel::Warn => entry_text.color(iced::Color::from_rgb(1.0, 0.75, 0.2)),
            _ => entry_text,
        };
        log_col = log_col.push(styled);
    }

    if entries.is_empty() {
        log_col = log_col.push(text("No log entries yet.").size(11));
    }

    let log_scroll = scrollable(log_col).height(Length::Fixed(300.0));

    #[allow(unused_mut)]
    let mut actions = row![
        button(text("Copy to Clipboard").size(12))
            .on_press(InterfaceMessage::CopyDebugLog)
            .padding([6, 14])
            .style(button::secondary),
        button(text("Save as File").size(12))
            .on_press(InterfaceMessage::SaveDebugLog)
            .padding([6, 14])
            .style(button::secondary),
    ]
    .spacing(8);

    #[cfg(any(debug_assertions, feature = "dev-tools"))]
    {
        let calibrate_btn = if controller.calibration.is_running() {
            button(text("Calibrating...").size(12))
                .padding([6, 14])
                .style(button::secondary)
        } else {
            button(text("Calibrate SysEx").size(12))
                .on_press(InterfaceMessage::CalibrateStart)
                .padding([6, 14])
                .style(button::primary)
        };
        let digest_btn = button(text("Run Digest").size(12))
            .on_press(InterfaceMessage::DigestRun)
            .padding([6, 14])
            .style(button::secondary);
        actions = actions.push(calibrate_btn);
        actions = actions.push(digest_btn);
    }

    let diagnostics = text(format!(
        "OS: {} {} | Entries: {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        count,
    ))
    .size(10);

    container(
        column![
            header_row,
            Space::new().height(8),
            container(log_scroll)
                .style(container::rounded_box)
                .width(Fill)
                .padding(8),
            Space::new().height(8),
            row![actions, Space::new().width(Fill), diagnostics].align_y(Center),
        ]
        .padding([12, 14])
        .width(Fill),
    )
    .style(container::rounded_box)
    .width(Fill)
    .into()
}

#[cfg(any(debug_assertions, feature = "dev-tools"))]
fn build_hex_viewer(dump: &[u8]) -> Element<'_, InterfaceMessage> {
    let preview_len = HEX_PREVIEW_BYTES.min(dump.len());
    let hex_text = sysex_parser::format_hex_dump(&dump[..preview_len]);
    let suffix = if dump.len() > preview_len {
        format!("\n... ({} more bytes)", dump.len() - preview_len)
    } else {
        String::new()
    };

    let hex_header = row![
        text(format!("Last SysEx Dump ({} bytes)", dump.len())).size(14),
        Space::new().width(Fill),
        button(text("Copy Full Hex Dump").size(11))
            .on_press(InterfaceMessage::CopyHexDump)
            .padding([4, 10])
            .style(button::secondary),
    ]
    .align_y(Center)
    .spacing(8);

    let mono_color = iced::Color::from_rgb(0.6, 0.8, 0.6);

    container(
        column![
            hex_header,
            Space::new().height(4),
            container(
                scrollable(
                    text(format!("{}{}", hex_text, suffix))
                        .size(10)
                        .color(mono_color)
                )
                .height(Length::Fixed(160.0))
            )
            .style(container::rounded_box)
            .width(Fill)
            .padding(8),
        ]
        .padding([12, 14])
        .spacing(2)
        .width(Fill),
    )
    .style(container::rounded_box)
    .width(Fill)
    .into()
}

fn build_about_section() -> Element<'static, InterfaceMessage> {
    let header = text("About").size(14);

    let version = text(format!(
        "FLOW 8 MIDI Controller v{}",
        env!("CARGO_PKG_VERSION")
    ))
    .size(12);

    let description = text(
        "A non-official cross-platform desktop MIDI controller for the Behringer FLOW 8 mixer.",
    )
    .size(11);

    let author = text("Author: Abel Rocha Espinosa").size(11);
    let license = text("License: GNU GPLv3").size(11);
    let repo_row = row![
        text("Repository:").size(11),
        Space::new().width(4),
        button(text("github.com/abelroes/flow-8-midi").size(11))
            .on_press(InterfaceMessage::OpenRepository)
            .padding([2, 6])
            .style(button::text),
    ]
    .align_y(Center);

    let donate_btn = button(text("Buy me a beer!").size(13).center())
        .on_press(InterfaceMessage::OpenDonation)
        .padding([10, 24])
        .style(move |theme, status| {
            let mut style = button::primary(theme, status);
            style.background = Some(iced::Background::Color(iced::Color {
                r: 0.96,
                g: 0.65,
                b: 0.14,
                a: 1.0,
            }));
            style.text_color = iced::Color::BLACK;
            style
        });

    let donate_row = column![
        text("If you find this useful, consider supporting the project:").size(11),
        Space::new().height(6),
        donate_btn,
    ];

    container(
        column![
            header,
            Space::new().height(8),
            version,
            Space::new().height(4),
            description,
            Space::new().height(6),
            author,
            license,
            repo_row,
            Space::new().height(8),
            donate_row,
        ]
        .padding([12, 14])
        .width(Fill),
    )
    .style(container::rounded_box)
    .width(Fill)
    .into()
}
