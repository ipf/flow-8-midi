#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod logger;
mod model;
pub mod service;
mod view;

use view::{
    device_select_page::view_device_select, eq_page::view_eq,
    fx_page::view_fx, mixer_page::view_mixer,
    nav_bar::view_nav_bar, sends_page::view_sends, settings_page::view_settings,
};
use image::load_from_memory;
use service::ble;
use service::midi::{
    connect_input_device, connect_to_device, create_sysex_channel, list_midi_input_devices,
    list_midi_output_devices, send_cc,
};
use service::midi_mapper::match_midi_command;
use service::sysex_parser;
use model::{
    channels::BusType,
    flow8::{FLOW8Controller, MeterMsg, RenameTarget},
    message::InterfaceMessage,
    page::Page,
};

use iced::{
    widget::{column, scrollable},
    window, Element, Fill, Size, Subscription, Theme,
};

fn open_url(url: &str) {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("open").arg(url).spawn();
    } else if cfg!(target_os = "windows") {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn();
    } else {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

fn clipboard_cmd() -> &'static str {
    if cfg!(target_os = "macos") {
        "pbcopy"
    } else if cfg!(target_os = "windows") {
        "clip"
    } else {
        "xclip"
    }
}

const APPLICATION_NAME: &str = "FLOW 8 Controller";
const PREFERRED_WIDTH: f32 = 1200.0;
const PREFERRED_HEIGHT: f32 = 700.0;
const MIN_WIDTH: f32 = 800.0;
const MIN_HEIGHT: f32 = 500.0;
static ICON: &[u8] = include_bytes!("../resources/icon.ico");

fn initial_window_size() -> Size {
    let (max_w, max_h) = screen_size_override().unwrap_or_else(available_screen_size);
    Size::new(PREFERRED_WIDTH.min(max_w), PREFERRED_HEIGHT.min(max_h))
}

fn screen_size_override() -> Option<(f32, f32)> {
    let val = std::env::var("FLOW8_SCREEN").ok()?;
    let (w, h) = val.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

#[cfg(target_os = "windows")]
fn available_screen_size() -> (f32, f32) {
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, pv: *mut Rect, flags: u32) -> i32;
        fn GetDpiForSystem() -> u32;
    }

    const SPI_GETWORKAREA: u32 = 0x0030;
    let mut rect = Rect { left: 0, top: 0, right: 0, bottom: 0 };

    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut rect, 0) };
    if ok != 0 {
        let dpi = unsafe { GetDpiForSystem() };
        let scale = if dpi >= 96 { dpi as f32 / 96.0 } else { 1.0 };

        let w = (rect.right - rect.left) as f32 / scale;
        let h = (rect.bottom - rect.top) as f32 / scale;

        if w > 100.0 && h > 100.0 {
            return (w - 16.0, h - 40.0);
        }
    }

    (PREFERRED_WIDTH, PREFERRED_HEIGHT)
}

#[cfg(not(target_os = "windows"))]
fn available_screen_size() -> (f32, f32) {
    (PREFERRED_WIDTH, PREFERRED_HEIGHT)
}

fn boot() -> FLOW8Controller {
    logger::init();
    let mut controller = FLOW8Controller::new();

    controller.ble_available = ble::is_ble_available();
    if controller.ble_available {
        log!("[APP] BLE adapter detected");
        controller.ble_status = ble::BleStatus::Disconnected;
    } else {
        log!("[APP] No BLE adapter — auto-sync unavailable");
    }

    try_connect_flow8(&mut controller);
    controller
}

fn theme(state: &FLOW8Controller) -> Theme {
    state.theme.clone()
}

fn main() -> iced::Result {
    let image = load_from_memory(ICON).unwrap();
    let icon = window::icon::from_rgba(image.as_bytes().to_vec(), image.width(), image.height()).unwrap();

    iced::application(boot, update, view)
        .title(APPLICATION_NAME)
        .theme(theme)
        .subscription(subscription)
        .antialiasing(true)
        .window(window::Settings {
            size: initial_window_size(),
            min_size: Some(Size::new(MIN_WIDTH, MIN_HEIGHT)),
            resizable: true,
            icon: Some(icon),
            position: window::Position::Centered,
            ..Default::default()
        })
        .run()
}

fn start_ble_connection(controller: &mut FLOW8Controller) {
    if !controller.ble_available {
        return;
    }
    if controller.ble_status == ble::BleStatus::Scanning
        || controller.ble_status == ble::BleStatus::Connecting
        || controller.ble_status == ble::BleStatus::Connected
    {
        return;
    }

    let (status_tx, status_rx) = std::sync::mpsc::channel();
    let (snapshot_tx, snapshot_rx) = std::sync::mpsc::channel();
    let (meter_tx, meter_rx) = std::sync::mpsc::channel();
    let (settings_tx, settings_rx) = std::sync::mpsc::channel();
    controller.ble_status_receiver = Some(status_rx);
    controller.snapshot_names_receiver = Some(snapshot_rx);
    controller.meter_receiver = Some(meter_rx);
    controller.ble_settings_receiver = Some(settings_rx);
    controller.ble_status = ble::BleStatus::Scanning;

    let conn_arc = controller.ble_connection.clone();
    std::thread::spawn(move || {
        match ble::connect_flow8_ble(status_tx, snapshot_tx, meter_tx, settings_tx) {
        Ok(conn) => {
            log!("[BLE] Connection established. Sending dump trigger...");
            let trigger_result = ble::send_dump_trigger(&conn);
            match trigger_result {
                Ok(_) => log!("[BLE] Initial dump trigger sent"),
                Err(e) => log_error!("[BLE] Initial dump trigger failed: {}", e),
            }

            if let Ok(mut guard) = conn_arc.lock() {
                *guard = Some(conn.clone());
            }

            // EXPERIMENT: locate device-setting bytes in the SysEx dump.
            // Toggle two settings over BLE (0x25) and re-dump; the diff
            // logging in apply_dump_to_controller names the offsets. Values
            // are restored (both expected to default to 0).
            std::thread::spawn(move || {
                use std::time::Duration;
                std::thread::sleep(Duration::from_millis(2500)); // baseline dump first
                let _ = ble::send_setting(&conn, 0x03, 1); // footswitch → snapshot
                std::thread::sleep(Duration::from_millis(600));
                let _ = ble::send_dump_trigger(&conn);
                std::thread::sleep(Duration::from_millis(1200));
                let _ = ble::send_setting(&conn, 0x03, 0); // restore
                let _ = ble::send_setting(&conn, 0x0E, 1); // main pad on
                std::thread::sleep(Duration::from_millis(600));
                let _ = ble::send_dump_trigger(&conn);
                std::thread::sleep(Duration::from_millis(1200));
                let _ = ble::send_setting(&conn, 0x0E, 0); // restore
                std::thread::sleep(Duration::from_millis(600));
                let _ = ble::send_dump_trigger(&conn);
                log!("[EXPERIMENT] settings diff sequence done");
            });
        }
        Err(e) => {
            log_error!("[BLE] Connection failed: {}", e);
        }
        }
    });
}

fn try_connect_flow8(controller: &mut FLOW8Controller) {
    let output_devices = list_midi_output_devices();
    let flow8_output = match output_devices.iter().find(|d| d.is_flow8) {
        Some(device) => device.clone(),
        None => {
            let msg = "FLOW 8 not found. Connect it via USB and try again.";
            log!("[APP] {}", msg);
            controller.connection_error = Some(msg.to_string());
            controller.current_page = Page::DeviceSelect;
            return;
        }
    };

    log!("[APP] Connecting to \"{}\" (port {})...", flow8_output.name, flow8_output.index);
    match connect_to_device(flow8_output.index) {
        Ok(conn) => {
            controller.midi_conn = Some(conn);
            controller.connected_device_name = Some(flow8_output.name);
            controller.connection_error = None;
            controller.current_page = Page::Mixer;

            let input_devices = list_midi_input_devices();
            if let Some(input_dev) = input_devices.iter().find(|d| d.is_flow8) {
                let (sysex_tx, sysex_rx) = create_sysex_channel();
                if let Ok(input_conn) = connect_input_device(input_dev.index, sysex_tx) {
                    controller.midi_input_conn = Some(input_conn);
                    controller.sysex_receiver = Some(sysex_rx);
                    log!("[APP] SysEx channel established");
                }
            }

            start_ble_connection(controller);
        }
        Err(e) => {
            log!("[APP] Connection failed: {}", e);
            controller.connection_error = Some(e);
            controller.current_page = Page::DeviceSelect;
        }
    }
}

/// Send the full FX setup block (0x11: params 1-3 + Main/Mon1/Mon2 routing)
/// for one FX slot, using the slot's current model state.
/// Channel mute (0x08) / solo (0x01): struct commands with [ch_id][state]
/// at cmd[24..25] (queueChannelMuteCommandPackets @0x7e2d0 /
/// queueChannelSoloCommandPackets @0x7e340).
fn send_channel_flag(controller: &FLOW8Controller, opcode: u8, ch_id: u8, state: bool) {
    let conn_arc = controller.ble_connection.clone();
    std::thread::spawn(move || {
        let Ok(guard) = conn_arc.lock() else {
            return;
        };
        if let Some(conn) = guard.as_ref() {
            if let Err(e) = ble::send_ble_struct_command(conn, opcode, &[ch_id, state as u8]) {
                log_warn!("[BLE] cmd 0x{:02X} failed: {}", opcode, e);
            } else {
                log!("[BLE] ch {} flag 0x{:02X} = {}", ch_id, opcode, state);
            }
        }
    });
}

fn send_fx_setup(controller: &FLOW8Controller, fx_idx: u8) {
    let Some(fx_id) = controller
        .buses
        .iter()
        .filter(|b| b.bus_type == BusType::Fx)
        .nth(fx_idx as usize)
        .map(|b| b.id)
    else {
        log_warn!("[BLE] FX setup: no FX bus in buses list for slot {}", fx_idx + 1);
        return;
    };
    let Some(f) = controller.fx_slots.get(fx_idx as usize) else {
        log_warn!("[BLE] FX setup: fx slot {} missing", fx_idx + 1);
        return;
    };
    let (p1, p2, p3) = (f.param1, f.param2, f.param3);
    let (m, m1, m2) = (f.route_main, f.route_mon1, f.route_mon2);
    let conn_arc = controller.ble_connection.clone();
    std::thread::spawn(move || {
        let Ok(guard) = conn_arc.lock() else {
            log_warn!("[BLE] FX setup: connection lock poisoned");
            return;
        };
        if let Some(conn) = guard.as_ref() {
            // 0x11 id = the device's channelId space (FX1=0x0C, FX2=0x0D) —
            // our old fx_id 10/11 = Mon1/Mon2 there
            if let Err(e) = ble::send_fx_setup(conn, 0x0A + (fx_id - 8), p1, p2, p3, m, m1, m2) {
                log_warn!("[BLE] FX setup failed: {}", e);
            } else {
                log!("[BLE] FX {} setup sent (params + routing)", fx_idx + 1);
            }
        } else {
            log_warn!("[BLE] FX setup: no BLE connection");
        }
    });
}

fn subscription(_controller: &FLOW8Controller) -> Subscription<InterfaceMessage> {
    iced::time::every(std::time::Duration::from_millis(50)).map(|_| InterfaceMessage::Tick)
}

fn update(controller: &mut FLOW8Controller, message: InterfaceMessage) {
    if let InterfaceMessage::BusLevel(_, bus_id, value) = &message {
        if *bus_id >= 10 {
            // FX return fader: ChannelMaster (0x06, dst = own id) — the
            // official app's path for every output fader including FX.
            // (The old "dead" proof was confounded: the FX bus meter is
            // post-routing, so a working return still metered silence while
            // FX routing to Main was off.) ChannelMaster id space:
            // Main=0x0F, Mon1/2=0x0A/0x0B, FX1/2=0x0C/0x0D. UI CC scale:
            // −70…+10 dB, cc 0 = OFF (−144).
            let cmd_id = 0x0A + (*bus_id - 8);
            let db = ble::level_cc_to_db(*value);
            let conn_arc = controller.ble_connection.clone();
            std::thread::spawn(move || {
                let Ok(guard) = conn_arc.lock() else {
                    return;
                };
                if let Some(conn) = guard.as_ref() {
                    if let Err(e) = ble::send_channel_master(conn, cmd_id, cmd_id, db) {
                        log_warn!("[BLE] FX return ChannelMaster failed: {}", e);
                    } else {
                        log!("[BLE] FX return 0x{:02X} = {:.1} dB", cmd_id, db);
                    }
                }
            });
            update_interface(controller, message);
            return;
        }
    }
    match_midi_command(&message, &mut controller.midi_conn);
    update_interface(controller, message);
}

fn try_ble_dump_trigger(controller: &FLOW8Controller) -> Option<Result<(), String>> {
    let conn_arc = controller.ble_connection.clone();
    // never block the UI thread: BlueZ write calls can stall up to 30 s
    let has_conn = conn_arc.lock().ok().map(|g| g.is_some()).unwrap_or(false);
    if !has_conn {
        return None;
    }
    std::thread::spawn(move || {
        let Ok(guard) = conn_arc.lock() else {
            return;
        };
        if let Some(conn) = guard.as_ref() {
            if let Err(e) = ble::send_dump_trigger(conn) {
                log_error!("[BLE] Dump trigger failed: {}", e);
            }
        }
    });
    Some(Ok(()))
}

fn view(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    if controller.current_page == Page::DeviceSelect {
        return view_device_select(controller);
    }

    let nav = view_nav_bar(controller);

    let page_content = match controller.current_page {
        Page::Mixer => view_mixer(controller),
        Page::Eq => view_eq(controller),
        Page::Sends => view_sends(controller),
        Page::Fx => view_fx(controller),
        Page::Settings => view_settings(controller),
        Page::DeviceSelect => unreachable!(),
    };

    // mixer fills the window exactly (no scrollbar); other pages scroll
    let page = match controller.current_page {
        Page::Mixer => page_content,
        _ => scrollable(page_content).height(Fill).into(),
    };

    column![nav, page]
        .width(Fill)
        .height(Fill)
        .into()
}

const SNAPSHOT_RESYNC_DELAY_MS: u64 = 500;

fn update_interface(controller: &mut FLOW8Controller, message: InterfaceMessage) {
    match message {
        InterfaceMessage::NavigateTo(page) => {
            controller.current_page = page;
        }
        InterfaceMessage::RetryConnection => {
            log!("[APP] Retrying connection...");
            controller.connection_error = None;
            try_connect_flow8(controller);
        }
        InterfaceMessage::Disconnect => {
            log!("[APP] Disconnecting BLE...");
            if let Ok(mut guard) = controller.ble_connection.lock() {
                if let Some(ref conn) = *guard {
                    ble::disconnect(conn);
                }
                *guard = None;
            }
            controller.ble_status = ble::BleStatus::Disconnected;
            controller.ble_status_receiver = None;
            controller.snapshot_names_receiver = None;
            controller.last_sync_time = None;
            controller.mark_all_unsynced();
        }

        InterfaceMessage::Mute(ch, _) => {
            let (id, state) = {
                let c = &mut controller.channels[ch as usize];
                c.is_muted = !c.is_muted;
                (c.id, c.is_muted)
            };
            send_channel_flag(controller, 0x08, id, state);
        }
        InterfaceMessage::Solo(ch, _) => {
            let (id, state) = {
                let c = &mut controller.channels[ch as usize];
                c.is_soloed = !c.is_soloed;
                (c.id, c.is_soloed)
            };
            send_channel_flag(controller, 0x01, id, state);
        }
        InterfaceMessage::Gain(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.channel_strip.gain = value;
            c.channel_strip.gain_synced = true;
        }
        InterfaceMessage::Level(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.channel_strip.level = value;
            c.channel_strip.level_synced = true;
        }
        InterfaceMessage::Balance(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.channel_strip.balance = value;
            c.channel_strip.balance_synced = true;
        }
        InterfaceMessage::LowCut(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.channel_strip.low_cut = value;
            c.channel_strip.low_cut_synced = true;
        }
        InterfaceMessage::Compressor(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.channel_strip.compressor = value;
            c.channel_strip.compressor_synced = true;
        }
        InterfaceMessage::PhantomPower(ch) => {
            let now = std::time::Instant::now();
            let c = &mut controller.channels[ch as usize];
            let is_double = c
                .phantom_last_click
                .map(|prev| now.duration_since(prev).as_millis() < 500)
                .unwrap_or(false);
            if is_double {
                c.phantom_pwr.is_on = !c.phantom_pwr.is_on;
                c.phantom_last_click = None;
                if let Some(conn) = controller.midi_conn.as_mut() {
                    send_cc(conn, ch, 12, if c.phantom_pwr.is_on { 127 } else { 0 });
                }
            } else {
                c.phantom_last_click = Some(now);
            }
        }

        InterfaceMessage::EqLow(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.four_band_eq.low = value;
            c.four_band_eq.low_synced = true;
        }
        InterfaceMessage::EqLowMid(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.four_band_eq.low_mid = value;
            c.four_band_eq.low_mid_synced = true;
        }
        InterfaceMessage::EqHiMid(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.four_band_eq.hi_mid = value;
            c.four_band_eq.hi_mid_synced = true;
        }
        InterfaceMessage::EqHi(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.four_band_eq.hi = value;
            c.four_band_eq.hi_synced = true;
        }

        InterfaceMessage::SendMon1(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.sends.mon1 = value;
            c.sends.mon1_synced = true;
        }
        InterfaceMessage::SendMon2(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.sends.mon2 = value;
            c.sends.mon2_synced = true;
        }
        InterfaceMessage::SendFx1(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.sends.fx1 = value;
            c.sends.fx1_synced = true;
        }
        InterfaceMessage::SendFx2(ch, value) => {
            let c = &mut controller.channels[ch as usize];
            c.sends.fx2 = value;
            c.sends.fx2_synced = true;
        }

        InterfaceMessage::BusLevel(bus_idx, _, value) => {
            let b = &mut controller.buses[bus_idx as usize];
            b.bus_strip.level = value;
            b.bus_strip.level_synced = true;
        }
        InterfaceMessage::BusBalance(bus_idx, _, value) => {
            let b = &mut controller.buses[bus_idx as usize];
            b.bus_strip.balance = value;
            b.bus_strip.balance_synced = true;
        }
        InterfaceMessage::BusLimiter(bus_idx, _, value) => {
            let b = &mut controller.buses[bus_idx as usize];
            b.bus_strip.limiter = value;
            b.bus_strip.limiter_synced = true;
        }
        InterfaceMessage::BusNineBandEq(bus_idx, _, band_index, value) => {
            let eq = &mut controller.buses[bus_idx as usize].nine_band_eq;
            match band_index {
                0 => eq.freq_62_hz = value,
                1 => eq.freq_125_hz = value,
                2 => eq.freq_250_hz = value,
                3 => eq.freq_500_hz = value,
                4 => eq.freq_1_khz = value,
                5 => eq.freq_2_khz = value,
                6 => eq.freq_4_khz = value,
                7 => eq.freq_8_khz = value,
                8 => eq.freq_16_khz = value,
                _ => {}
            }
            if (band_index as usize) < 9 {
                eq.bands_synced[band_index as usize] = true;
            }
        }
        InterfaceMessage::MonitorTab(tab) => {
            controller.monitor_tab = tab;
        }

        InterfaceMessage::FxPreset(fx_id, preset) => {
            let f = &mut controller.fx_slots[fx_id as usize];
            f.preset = preset;
            f.preset_synced = true;
            if let Some(fx_bus_id) = controller
                .buses
                .iter()
                .filter(|b| b.bus_type == BusType::Fx)
                .nth(fx_id as usize)
                .map(|b| b.id)
            {
                // 0x31 takes the device's channelId space (FX1=0x0C, FX2=0x0D
                // — native queueFXLoadPresetCommandPackets passes the Kotlin's
                // outputBus.channelId through unmapped) and a 0-based preset
                // index (the 1..=16 dump values were the device's pushback).
                // Our old fx_id 10/11 = Mon1/Mon2 in that space — every
                // preset load landed on the wrong bank.
                let fx_ch_id = 0x0A + (fx_bus_id - 8);
                let conn_arc = controller.ble_connection.clone();
                std::thread::spawn(move || {
                    let Ok(guard) = conn_arc.lock() else {
                        return;
                    };
                    if let Some(conn) = guard.as_ref() {
                        if let Err(e) = ble::send_fx_preset(conn, fx_ch_id, preset) {
                            log_warn!("[BLE] FX preset load failed: {}", e);
                        } else {
                            log!("[BLE] FX {} (0x{:02X}) preset {} sent", fx_id + 1, fx_ch_id, preset);
                        }
                    }
                });

                // the device resets FX state on a preset load (sends
                // dump-proven; routing wiped too — the user found FX→Main
                // off) — re-assert routing + params via FXSetup and the
                // stored return levels once the load settles
                let setups: Vec<(u8, u8, u8, u8, bool, bool, bool)> = (0..2)
                    .filter_map(|i| {
                        let f = controller.fx_slots.get(i)?;
                        let bus = controller
                            .buses
                            .iter()
                            .filter(|b| b.bus_type == BusType::Fx)
                            .nth(i)?;
                        Some((
                            0x0A + (bus.id - 8),
                            f.param1,
                            f.param2,
                            f.param3,
                            f.route_main,
                            f.route_mon1,
                            f.route_mon2,
                        ))
                    })
                    .collect();
                let returns: Vec<(u8, u8)> = controller
                    .buses
                    .iter()
                    .filter(|b| b.bus_type == BusType::Fx)
                    .map(|b| (0x0A + (b.id - 8), b.bus_strip.level))
                    .collect();
                let conn_arc = controller.ble_connection.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    let Ok(guard) = conn_arc.lock() else {
                        return;
                    };
                    if let Some(conn) = guard.as_ref() {
                        for (ch_id, p1, p2, p3, m, m1, m2) in setups {
                            if let Err(e) =
                                ble::send_fx_setup(conn, ch_id, p1, p2, p3, m, m1, m2)
                            {
                                log_warn!("[BLE] FX setup restore failed: {}", e);
                            }
                        }
                        for (ch_id, cc) in returns {
                            let db = ble::level_cc_to_db(cc);
                            if let Err(e) = ble::send_channel_master(conn, ch_id, ch_id, db) {
                                log_warn!("[BLE] FX return restore failed: {}", e);
                            }
                        }
                        log!("[BLE] FX routing/params/returns restored after preset load");
                    }
                });
            }
        }
        InterfaceMessage::FxParam1(fx_id, value) => {
            let f = &mut controller.fx_slots[fx_id as usize];
            f.param1 = value;
            f.param1_synced = true;
        }
        InterfaceMessage::FxParam1Send(fx_id) => {
            send_fx_setup(controller, fx_id);
        }
        InterfaceMessage::FxParam2(fx_id, value) => {
            let f = &mut controller.fx_slots[fx_id as usize];
            f.param2 = value;
            f.param2_synced = true;
            send_fx_setup(controller, fx_id);
        }
        InterfaceMessage::FxRouting(fx_idx, to_main, to_mon1, to_mon2) => {
            if let Some(f) = controller.fx_slots.get_mut(fx_idx as usize) {
                f.route_main = to_main;
                f.route_mon1 = to_mon1;
                f.route_mon2 = to_mon2;
            }
            send_fx_setup(controller, fx_idx);
        }
        InterfaceMessage::FxMute => {
            controller.fx_muted = !controller.fx_muted;
            if let Some(conn) = controller.midi_conn.as_mut() {
                send_cc(conn, 15, 1, if controller.fx_muted { 127 } else { 0 });
            }
        }
        InterfaceMessage::TapTempo => {
            let now = std::time::Instant::now();
            // keep taps within 3 s of each other, at most 5 (official-app behavior)
            controller
                .tap_times
                .retain(|t| now.duration_since(*t).as_secs_f32() < 3.0);
            controller.tap_times.push(now);
            if controller.tap_times.len() > 5 {
                controller.tap_times.remove(0);
            }
            if let Some(bpm) = controller.current_tap_bpm() {
                let conn_arc = controller.ble_connection.clone();
                std::thread::spawn(move || {
                    let Ok(guard) = conn_arc.lock() else {
                        return;
                    };
                    if let Some(conn) = guard.as_ref() {
                        if let Err(e) = ble::send_fx_tempo(conn, bpm) {
                            log_warn!("[BLE] FX tempo failed: {}", e);
                        } else {
                            log!("[BLE] FX tempo set to {} BPM", bpm);
                        }
                    }
                });
            }
        }

        InterfaceMessage::DeleteSnapshot(slot) => {
            let now = std::time::Instant::now();
            let confirmed = controller
                .delete_pending
                .map(|(s, t)| s == slot && now.duration_since(t).as_millis() < 800)
                .unwrap_or(false);
            if !confirmed {
                controller.delete_pending = Some((slot, now));
                log!("[APP] Delete snapshot {}: click again to confirm", slot + 1);
                return;
            }
            controller.delete_pending = None;
            log!("[APP] Deleting snapshot {}", slot + 1);
            if let Some(name) = controller.snapshot_names.get_mut(slot) {
                *name = None;
            }
            let conn_arc = controller.ble_connection.clone();
            let (snap_tx, snap_rx) = std::sync::mpsc::channel();
            controller.snapshot_names_receiver = Some(snap_rx);
            std::thread::spawn(move || {
                let Ok(guard) = conn_arc.lock() else {
                    return;
                };
                let Some(conn) = guard.as_ref() else {
                    log_warn!("[BLE] Delete snapshot: not connected");
                    return;
                };
                match ble::send_snapshot_delete(conn, slot as u8) {
                    Ok(_) => log!("[BLE] Snapshot delete sent"),
                    Err(e) => log_warn!("[BLE] Snapshot delete failed: {}", e),
                }
                let names = ble::refresh_snapshot_names(conn);
                let _ = snap_tx.send(names);
            });
        }

        InterfaceMessage::SetDeviceSetting(id, value) => {
            controller.ble_settings[id as usize] = Some(value);
            let conn_arc = controller.ble_connection.clone();
            std::thread::spawn(move || {
                let Ok(guard) = conn_arc.lock() else {
                    return;
                };
                if let Some(conn) = guard.as_ref() {
                    match ble::send_setting(conn, id, value) {
                        Ok(_) => log!("[BLE] Setting {:02X} <- {:02X}", id, value),
                        Err(e) => log_warn!("[BLE] Setting {:02X} failed: {}", id, e),
                    }
                }
            });
        }

        InterfaceMessage::Tick => {
            controller.tick_counter = controller.tick_counter.wrapping_add(1);

            // clear the snapshot-delete confirm window
            if let Some((_, t)) = controller.delete_pending {
                if t.elapsed().as_millis() >= 800 {
                    controller.delete_pending = None;
                }
            }

            let pending: Vec<Vec<u8>> = controller
                .sysex_receiver
                .as_ref()
                .map(|rx| std::iter::from_fn(|| rx.try_recv().ok()).collect())
                .unwrap_or_default();

            for data in pending {
                log!("[SYSEX] Received data ({} bytes)", data.len());

                #[cfg(any(debug_assertions, feature = "dev-tools"))]
                if controller.calibration.is_running() {
                    controller.calibration.on_dump_received(data);
                    continue;
                }

                if let Some(dump) = sysex_parser::validate_sysex_dump(&data) {
                    sysex_parser::apply_dump_to_controller(&dump, controller);
                }
            }

            #[cfg(any(debug_assertions, feature = "dev-tools"))]
            if controller.calibration.is_running() {
                let action = controller.calibration.tick();
                if let Some((midi_ch, cc, value)) = action.send_cc {
                    if let Some(conn) = controller.midi_conn.as_mut() {
                        send_cc(conn, midi_ch, cc, value);
                    }
                }
                if let Some((midi_ch, program)) = action.send_pc {
                    if let Some(conn) = controller.midi_conn.as_mut() {
                        service::midi::send_program_change(conn, midi_ch, program);
                    }
                }
                if action.trigger_dump {
                    match try_ble_dump_trigger(controller) {
                        Some(Ok(_)) => {}
                        Some(Err(e)) => log_error!("[CALIB] Dump trigger failed: {}", e),
                        None => log_warn!("[CALIB] No BLE connection for dump trigger"),
                    }
                }
            }

            if let Some(ref ble_rx) = controller.ble_status_receiver {
                while let Ok(status) = ble_rx.try_recv() {
                    controller.ble_status = status;
                    log!("[BLE] Status: {}", status);
                }
            }

            if let Some(ref snap_rx) = controller.snapshot_names_receiver {
                while let Ok(names) = snap_rx.try_recv() {
                    log!("[BLE] Received {} snapshot names", names.len());
                    controller.snapshot_names = names;
                }
            }

            // device settings read back at connect (0x26 probe)
            if let Some(ref settings_rx) = controller.ble_settings_receiver {
                while let Ok((id, value)) = settings_rx.try_recv() {
                    log!("[BLE] Setting {:02X} read back: {:02X}", id, value);
                    controller.ble_settings[id as usize] = Some(value);
                }
            }

            // VU meters: display slides toward the last-frame level so motion
            // is smooth between the device's ~400 ms frame cadence (factors
            // per 50 ms tick; 200 ms cadence equivalents: 0.90 / 0.995).
            for ch in &mut controller.channels {
                if ch.meter_target > ch.meter {
                    ch.meter += (ch.meter_target - ch.meter) * 0.6;
                } else {
                    ch.meter *= 0.975;
                }
            }
            for bus in &mut controller.buses {
                if bus.meter_target > bus.meter {
                    bus.meter += (bus.meter_target - bus.meter) * 0.6;
                } else {
                    bus.meter *= 0.9988;
                }
            }
            if let Some(ref meter_rx) = controller.meter_receiver {
                while let Ok(msg) = meter_rx.try_recv() {
                    match msg {
                        MeterMsg::Frame(values, out_rot, gr) => {
                            // floats 0-9 (official index table): 0-3 = inputs 1-4,
                            // 4/5 = input5 L/R, 6/7 = input6 L/R, 8/9 = USB/BT L/R
                            for (i, ch) in controller.channels.iter_mut().enumerate() {
                                let db = match i {
                                    0..=3 => values.get(i).copied(),
                                    4 => values.get(4).zip(values.get(5)).map(|(l, r)| l.max(*r)),
                                    5 => values.get(6).zip(values.get(7)).map(|(l, r)| l.max(*r)),
                                    _ => values.get(8).zip(values.get(9)).map(|(l, r)| l.max(*r)),
                                };
                                if let Some(db) = db {
                                    let linear = crate::service::ble::meter_db_to_linear(db);
                                    ch.meter_target = linear;
                                }
                                // GR: our channel i == the device's input
                                // channel idx (0..=6), gr[7] = the metered
                                // output
                                ch.gr = gr.get(i as usize).copied().unwrap_or(0);
                            }
                            // floats 10/11 = L/R of the rotated metered output
                            // (rotation 0/1/2 → bus id 7/8/9 = Main/Mon 1/Mon 2)
                            if let (Some(l), Some(r)) = (values.get(10), values.get(11)) {
                                let db = l.max(*r);
                                let bus_id = 7 + out_rot as u8;
                                if let Some(bus) =
                                    controller.buses.iter_mut().find(|b| b.id == bus_id)
                                {
                                    bus.meter_target =
                                        crate::service::ble::meter_db_to_linear(db);
                                    bus.gr = gr.get(7).copied().unwrap_or(0);
                                }
                            }
                        }
                        MeterMsg::Fader { input, output, value } => {
                            // 0x06 push = fader position, NOT a signal level:
                            // mirror it into the UI. The value byte is fix8
                            // (F48 dB index, selector 3) — convert to the
                            // 1..=127 CC scale first (raw 190 = −0.2 dB, not
                            // +50). Id space (official app: LRBusChannelId
                            // 15, AUX 10/11, FX 12/13): a bus id as `input`
                            // = that bus's own fader (FX return level), a
                            // channel id = its send level to `output`.
                            let value = crate::service::ble::fix8_to_level_cc(value);
                            let bus_fader = match input {
                                0x0F => Some(7),
                                0x0A => Some(8),
                                0x0B => Some(9),
                                0x0C => Some(10),
                                0x0D => Some(11),
                                0x18 => Some(8),
                                0x19 => Some(9),
                                _ => None,
                            };
                            if let Some(bus_id) = bus_fader {
                                if let Some(bus) = controller
                                    .buses
                                    .iter_mut()
                                    .find(|b| b.id == bus_id)
                                {
                                    bus.bus_strip.level = value;
                                    bus.bus_strip.level_synced = true;
                                }
                            } else if let Some(ch) =
                                controller.channels.get_mut((input & 0x07) as usize)
                            {
                                match output {
                                    0x0F => {
                                        ch.channel_strip.level = value;
                                        ch.channel_strip.level_synced = true;
                                    }
                                    0x0A | 0x18 => {
                                        ch.sends.mon1 = value;
                                        ch.sends.mon1_synced = true;
                                    }
                                    0x0B | 0x19 => {
                                        ch.sends.mon2 = value;
                                        ch.sends.mon2_synced = true;
                                    }
                                    0x0C => {
                                        ch.sends.fx1 = value;
                                        ch.sends.fx1_synced = true;
                                    }
                                    0x0D => {
                                        ch.sends.fx2 = value;
                                        ch.sends.fx2_synced = true;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }

            // clip latch: red dot above the fader for 2 s once the level hits 0 dBFS
            // (latched on the raw frame target — the display lags it by design)
            for ch in &mut controller.channels {
                if ch.meter_target >= 0.99
                    && ch
                        .clip_time
                        .map(|t| t.elapsed().as_millis() > 2000)
                        .unwrap_or(true)
                {
                    ch.clip_time = Some(std::time::Instant::now());
                }
            }

            if let Some(pending_at) = controller.snapshot_resync_at {
                if pending_at.elapsed().as_millis() >= SNAPSHOT_RESYNC_DELAY_MS as u128 {
                    controller.snapshot_resync_at = None;
                    match try_ble_dump_trigger(controller) {
                        Some(Ok(_)) => {
                            controller.last_sync_time = Some(std::time::Instant::now());
                            log!("[APP] Post-snapshot resync triggered");
                        }
                        Some(Err(e)) => log_error!("[APP] Post-snapshot resync failed: {}", e),
                        None => log_warn!("[APP] No BLE connection for post-snapshot resync"),
                    }
                }
            }

            if controller.ble_status == ble::BleStatus::Connected {
                if let Some(interval_secs) = controller.sync_interval.as_secs() {
                    let should_sync = controller
                        .last_sync_time
                        .map(|t| t.elapsed().as_secs() >= interval_secs)
                        .unwrap_or(true);
                    if should_sync {
                        match try_ble_dump_trigger(controller) {
                            Some(Ok(_)) => {
                                controller.last_sync_time = Some(std::time::Instant::now());
                                log!("[APP] Auto-sync triggered");
                            }
                            Some(Err(e)) => log_error!("[APP] Auto-sync dump failed: {}", e),
                            None => {}
                        }
                    }
                }
            }
        }

        InterfaceMessage::BleConnect => {
            let now = std::time::Instant::now();
            let is_double = controller
                .ble_last_click
                .map(|prev| now.duration_since(prev).as_millis() < 500)
                .unwrap_or(false);
            if is_double {
                controller.ble_last_click = None;
                start_ble_connection(controller);
            } else {
                controller.ble_last_click = Some(now);
            }
        }

        InterfaceMessage::BleRequestDump => {
            controller.sync_last_click = Some(std::time::Instant::now());
            match try_ble_dump_trigger(controller) {
                Some(Ok(_)) => {
                    controller.last_sync_time = Some(std::time::Instant::now());
                    log!("[APP] Dump trigger sent via BLE");
                }
                Some(Err(e)) => log_error!("[APP] Dump trigger failed: {}", e),
                None => log_warn!("[APP] No BLE connection for dump trigger"),
            }
        }

        InterfaceMessage::LoadSnapshot(n) => {
            log!("[APP] Loading snapshot {}", n + 1);
            controller.mark_all_unsynced();
            controller.snapshot_resync_at = Some(std::time::Instant::now());
            // recall over BLE (the MIDI Program Change goes out via the mapper;
            // BLE works even when the USB MIDI path is wedged)
            let conn_arc = controller.ble_connection.clone();
            std::thread::spawn(move || {
                let Ok(guard) = conn_arc.lock() else {
                    return;
                };
                if let Some(conn) = guard.as_ref() {
                    match ble::send_snapshot_load(conn, n) {
                        Ok(_) => log!("[BLE] Snapshot load sent"),
                        Err(e) => log_warn!("[BLE] Snapshot load failed: {}", e),
                    }
                }
            });
        }
        InterfaceMessage::RenameStartChannel(ch_id) => {
            let name = controller
                .channels
                .iter()
                .find(|c| c.id == ch_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            controller.rename_target = Some(RenameTarget::Channel(ch_id));
            controller.rename_buffer = name;
        }
        InterfaceMessage::RenameStartSnapshot(slot) => {
            controller.rename_target = Some(RenameTarget::Snapshot(slot));
            controller.rename_buffer = controller
                .snapshot_names
                .get(slot)
                .cloned()
                .flatten()
                .unwrap_or_default();
        }
        InterfaceMessage::SnapshotSaveStart(slot) => {
            controller.rename_target = Some(RenameTarget::SnapshotSave(slot));
            controller.rename_buffer = controller
                .snapshot_names
                .get(slot)
                .cloned()
                .flatten()
                .unwrap_or_default();
        }
        InterfaceMessage::RenameChanged(value) => {
            controller.rename_buffer = value;
        }
        InterfaceMessage::RenameCancel => {
            controller.rename_target = None;
            controller.rename_buffer.clear();
        }
        InterfaceMessage::RenameSubmit => {
            let Some(target) = controller.rename_target.take() else {
                return;
            };
            let name = controller.rename_buffer.clone();
            controller.rename_buffer.clear();

            if let RenameTarget::Channel(ch_id) = target {
                if let Some(ch) = controller.channels.iter_mut().find(|c| c.id == ch_id) {
                    ch.name = name.clone();
                    ch.name_synced = true;
                }
            }
            if let RenameTarget::SnapshotSave(slot) = target {
                // note when this slot was saved (display only; the device
                // does not report save dates)
                let slot = slot as usize;
                if slot < controller.snapshot_dates.len() {
                    controller.snapshot_dates[slot] =
                        Some(chrono::Local::now().format("%d.%m.%Y %H:%M").to_string());
                }
            }

            // snapshot ops get a fresh names channel the Tick loop consumes
            let (snap_tx, snap_rx) = std::sync::mpsc::channel();
            if matches!(target, RenameTarget::Snapshot(_) | RenameTarget::SnapshotSave(_)) {
                controller.snapshot_names_receiver = Some(snap_rx);
            }

            let conn_arc = controller.ble_connection.clone();
            std::thread::spawn(move || {
                let Ok(guard) = conn_arc.lock() else {
                    log_warn!("[BLE] Rename: connection mutex poisoned");
                    return;
                };
                let Some(conn) = guard.as_ref() else {
                    log_warn!("[BLE] Rename: not connected");
                    return;
                };
                let result = match target {
                    RenameTarget::Channel(ch_id) => {
                        log!("[BLE] Renaming channel {} to \"{}\"", ch_id + 1, name);
                        ble::send_channel_rename(conn, ch_id, &name)
                    }
                    RenameTarget::Snapshot(slot) => {
                        log!("[BLE] Renaming snapshot {} to \"{}\"", slot + 1, name);
                        ble::send_snapshot_rename(conn, slot as u8, &name)
                    }
                    RenameTarget::SnapshotSave(slot) => {
                        log!("[BLE] Saving mixer state to snapshot {} as \"{}\"", slot + 1, name);
                        ble::send_snapshot_save(conn, slot as u8, &name)
                    }
                };
                match result {
                    Ok(_) => log!("[BLE] Rename/save command sent"),
                    Err(e) => log_warn!("[BLE] Rename/save failed: {}", e),
                }
                if matches!(target, RenameTarget::Snapshot(_) | RenameTarget::SnapshotSave(_)) {
                    let names = ble::refresh_snapshot_names(conn);
                    let _ = snap_tx.send(names);
                }
            });
        }
        InterfaceMessage::ResetMixer => {
            log!("[APP] Resetting mixer to default");
            controller.mark_all_unsynced();
            controller.snapshot_resync_at = Some(std::time::Instant::now());
        }

        InterfaceMessage::ThemeChanged(theme) => {
            controller.theme = theme;
        }
        InterfaceMessage::SyncIntervalChanged(interval) => {
            controller.sync_interval = interval;
            log!("[APP] Sync interval changed to {}", interval);
        }
        InterfaceMessage::OpenManual => {
            open_url("https://github.com/abelroes/flow-8-midi/blob/main/docs/MANUAL.md");
        }
        InterfaceMessage::OpenRepository => {
            open_url("https://github.com/abelroes/flow-8-midi");
        }
        InterfaceMessage::OpenDonation => {
            open_url("https://buymeacoffee.com/abelroes");
        }
        InterfaceMessage::CopyDebugLog => {
            let export = logger::export_log();
            if let Ok(mut child) = std::process::Command::new(clipboard_cmd())
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(ref mut stdin) = child.stdin {
                    use std::io::Write;
                    let _ = stdin.write_all(export.as_bytes());
                }
                let _ = child.wait();
                log!("[APP] Debug log copied to clipboard");
            } else {
                log_warn!("[APP] Could not access clipboard");
            }
        }
        #[cfg(any(debug_assertions, feature = "dev-tools"))]
        InterfaceMessage::CopyHexDump => {
            if let Some(ref dump) = controller.last_sysex_dump {
                let hex = sysex_parser::format_hex_dump(dump);
                if let Ok(mut child) = std::process::Command::new(clipboard_cmd())
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                {
                    if let Some(ref mut stdin) = child.stdin {
                        use std::io::Write;
                        let _ = stdin.write_all(hex.as_bytes());
                    }
                    let _ = child.wait();
                    log!("[APP] Hex dump copied to clipboard ({} bytes)", dump.len());
                }
            } else {
                log_warn!("[APP] No SysEx dump available to copy");
            }
        }
        InterfaceMessage::SaveDebugLog => {
            let export = logger::export_log();
            let filename = format!(
                "flow8-debug-{}.log",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            );
            match std::fs::write(&filename, &export) {
                Ok(_) => log!("[APP] Debug log saved to {}", filename),
                Err(e) => log_error!("[APP] Failed to save debug log: {}", e),
            }
        }
        #[cfg(any(debug_assertions, feature = "dev-tools"))]
        InterfaceMessage::CalibrateStart => {
            if controller.calibration.is_running() {
                log_warn!("[APP] Calibration already in progress");
                return;
            }
            if controller.midi_conn.is_none() {
                log_warn!("[APP] Cannot calibrate: no MIDI connection");
                return;
            }
            let has_ble = controller
                .ble_connection
                .lock()
                .ok()
                .map(|g| g.is_some())
                .unwrap_or(false);
            if !has_ble {
                log_warn!("[APP] Cannot calibrate: no BLE connection (needed to trigger dumps)");
                return;
            }
            log!("[APP] Starting SysEx calibration...");
            controller.calibration.start();
        }
        #[cfg(any(debug_assertions, feature = "dev-tools"))]
        InterfaceMessage::DigestRun => {
            log!("[APP] Running file-based digest...");
            match service::sysex_calibration::run_file_based_digest() {
                Ok(summary) => log!("[APP] Digest complete: {}", summary),
                Err(e) => log_warn!("[APP] Digest failed: {}", e),
            }
        }
    }
}
