use crate::service::ble::BleStatus;
use crate::model::{flow8::FLOW8Controller, message::InterfaceMessage, page::Page};
use iced::{
    widget::{button, container, row, text, tooltip, tooltip::Position, Space},
    Center, Element, Fill,
};

// Only the mixer: EQ, Sends and FX pages are gone — the mixer strips cover
// EQ (4-band per input, 9-band per bus), fx sends, FX returns, and the FX
// controls (presets/params/routing/mute/tap) now live in the FX strips.
const NAV_PAGES: [Page; 1] = [Page::Mixer];

pub fn view_nav_bar(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    let mut tabs = row![].spacing(4).align_y(Center);

    for page in NAV_PAGES {
        let label = text(format!("{}", page)).size(13);
        let btn = if page == controller.current_page {
            button(label).padding([6, 16])
        } else {
            button(label)
                .on_press(InterfaceMessage::NavigateTo(page))
                .padding([6, 16])
                .style(button::secondary)
        };
        tabs = tabs.push(btn);
    }

    tabs = tabs.push(Space::new().width(Fill));

    let bt_led = build_ble_indicator(controller);
    tabs = tabs.push(bt_led);
    tabs = tabs.push(Space::new().width(4));

    let ble_active = matches!(
        controller.ble_status,
        BleStatus::Connected | BleStatus::Scanning | BleStatus::Connecting | BleStatus::Authenticating
    );

    // one button, label follows the state: Connect (primary) when idle,
    // Disconnect (danger) when a link exists; busy states (scanning,
    // connecting) keep it disabled — there is no link to tear down yet
    let (btn_label, tip) = if ble_active {
        ("Disconnect", "Disconnect BLE from mixer")
    } else {
        ("Connect", "Connect to the FLOW 8 via Bluetooth")
    };
    let btn_base = button(text(btn_label).size(11)).padding([4, 10]);
    let connect_btn = if ble_active {
        if controller.ble_status == BleStatus::Connected {
            btn_base.style(button::danger).on_press(InterfaceMessage::Disconnect)
        } else {
            btn_base.style(button::danger)
        }
    } else {
        btn_base.style(button::primary).on_press(InterfaceMessage::BleConnectNow)
    };

    tabs = tabs.push(
        tooltip(
            connect_btn,
            container(text(tip).size(11))
                .padding(6)
                .style(container::rounded_box),
            Position::Bottom,
        )
        .gap(4),
    );

    tabs = tabs.push(Space::new().width(8));

    let settings_btn = if controller.current_page == Page::Settings {
        button(text("\u{2699}").size(13)).padding([4, 8])
    } else {
        button(text("\u{2699}").size(13))
            .on_press(InterfaceMessage::NavigateTo(Page::Settings))
            .padding([4, 8])
            .style(button::secondary)
    };
    tabs = tabs.push(
        tooltip(
            settings_btn,
            container(text("Settings").size(11))
                .padding(6)
                .style(container::rounded_box),
            tooltip::Position::Bottom,
        )
        .gap(4),
    );

    tabs = tabs.push(Space::new().width(4));

    let about_btn = if controller.current_page == Page::About {
        button(text("\u{2753}").size(13)).padding([4, 8])
    } else {
        button(text("\u{2753}").size(13))
            .on_press(InterfaceMessage::NavigateTo(Page::About))
            .padding([4, 8])
            .style(button::secondary)
    };
    tabs = tabs.push(
        tooltip(
            about_btn,
            container(text("About (app info, manual, debug log)").size(11))
                .padding(6)
                .style(container::rounded_box),
            tooltip::Position::Bottom,
        )
        .gap(4),
    );

    container(tabs)
        .width(Fill)
        .padding([8, 12])
        .into()
}

const BLE_CONNECTED_COLOR: iced::Color = iced::Color {
    r: 0.3,
    g: 0.6,
    b: 1.0,
    a: 1.0,
};

const BLE_DISCONNECTED_COLOR: iced::Color = iced::Color {
    r: 0.4,
    g: 0.4,
    b: 0.4,
    a: 1.0,
};

const BLE_BUSY_COLOR: iced::Color = iced::Color {
    r: 1.0,
    g: 0.8,
    b: 0.2,
    a: 1.0,
};

const BLE_ERROR_COLOR: iced::Color = iced::Color {
    r: 1.0,
    g: 0.3,
    b: 0.3,
    a: 1.0,
};

const BLE_BLINK_DIM: iced::Color = iced::Color {
    r: 0.5,
    g: 0.4,
    b: 0.1,
    a: 0.4,
};

fn build_ble_indicator(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    let is_busy = matches!(
        controller.ble_status,
        BleStatus::Scanning | BleStatus::Connecting | BleStatus::Authenticating
    );

    let color = match controller.ble_status {
        BleStatus::Connected => BLE_CONNECTED_COLOR,
        _ if is_busy => {
            if controller.tick_counter % 5 < 3 {
                BLE_BUSY_COLOR
            } else {
                BLE_BLINK_DIM
            }
        }
        BleStatus::Error => BLE_ERROR_COLOR,
        _ => BLE_DISCONNECTED_COLOR,
    };

    let tip = match controller.ble_status {
        BleStatus::Connected => "BLE connected",
        BleStatus::Scanning => "Scanning for FLOW 8 LE...",
        BleStatus::Connecting => "Connecting to FLOW 8 LE...",
        BleStatus::Authenticating => "Authenticating...",
        BleStatus::Error => "BLE error — double-click to retry",
        BleStatus::Disconnected => "BLE disconnected — double-click to reconnect",
        BleStatus::Unavailable => "No Bluetooth adapter",
    };

    let content = row![
        text("\u{25CF}").size(10).color(color),
        text("BT").size(10),
    ]
    .spacing(3)
    .align_y(Center);

    tooltip(
        button(content)
            .on_press(InterfaceMessage::BleConnect)
            .padding([4, 8])
            .style(button::text),
        container(text(tip).size(11))
            .padding(6)
            .style(container::rounded_box),
        Position::Bottom,
    )
    .gap(4)
    .into()
}
