use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use std::pin::Pin;
use std::sync::mpsc;
use std::time::Duration;
use uuid::Uuid;

use crate::{log, log_debug, log_error, log_warn};

const SERVICE_UUID: &str = "14839ad4-8d7e-415c-9a42-167340cf2339";
const CHARACTERISTIC_UUID: &str = "0034594a-a8e7-4b1a-a6b1-cd5243059a57";
const FLOW8_NAME_KEYWORD: &str = "FLOW";
const SCAN_TIMEOUT: Duration = Duration::from_secs(12);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

const AUTH_PACKET: [u8; 19] = [
    0x39, 0x01, 0xFD, 0x06, 0x2B, 0x06, 0x39, 0xF1, 0x7F, 0xE7, 0xB7, 0x27, 0x8B, 0x8F, 0x35,
    0x5A, 0x49, 0x5C, 0x2A,
];
const SESSION_START_PACKET: [u8; 3] = [0x37, 0x01, 0x38];
const CONFIG_REQUEST_PACKET: [u8; 3] = [0x07, 0x01, 0x08];
const DUMP_TRIGGER_PACKET: [u8; 3] = [0x4B, 0x01, 0x4C];

const SNAPSHOT_RESPONSE_TYPE: u8 = 0x27;
const STATE_DUMP_TYPE: u8 = 0x38;

/// the shared GATT notification stream (one subscribe per connection; every
/// consumer — handshake, snapshot fetch, meter loop — reads from it in turn)
type NotifyStream = Pin<Box<dyn tokio_stream::Stream<Item = ValueNotification> + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleStatus {
    Unavailable,
    Scanning,
    Connecting,
    Authenticating,
    Connected,
    Disconnected,
    Error,
}

impl std::fmt::Display for BleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BleStatus::Unavailable => write!(f, "BT Unavailable"),
            BleStatus::Scanning => write!(f, "BT Scanning..."),
            BleStatus::Connecting => write!(f, "BT Connecting..."),
            BleStatus::Authenticating => write!(f, "BT Authenticating..."),
            BleStatus::Connected => write!(f, "BT Connected"),
            BleStatus::Disconnected => write!(f, "BT Disconnected"),
            BleStatus::Error => write!(f, "BT Error"),
        }
    }
}

#[derive(Clone)]
pub struct BleConnection {
    pub peripheral: Peripheral,
    pub characteristic: Characteristic,
}

fn service_uuid() -> Uuid {
    Uuid::parse_str(SERVICE_UUID).unwrap()
}

fn characteristic_uuid() -> Uuid {
    Uuid::parse_str(CHARACTERISTIC_UUID).unwrap()
}

pub fn is_ble_available() -> bool {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(_) => return false,
    };

    rt.block_on(async {
        let manager = match Manager::new().await {
            Ok(m) => m,
            Err(_) => return false,
        };
        let adapters = match manager.adapters().await {
            Ok(a) => a,
            Err(_) => return false,
        };
        !adapters.is_empty()
    })
}

async fn get_adapter() -> Result<Adapter, String> {
    let manager = Manager::new()
        .await
        .map_err(|e| format!("BLE manager error: {}", e))?;

    let adapters = manager
        .adapters()
        .await
        .map_err(|e| format!("BLE adapter list error: {}", e))?;

    adapters
        .into_iter()
        .next()
        .ok_or_else(|| "No BLE adapter found".to_string())
}

async fn scan_for_flow8(adapter: &Adapter) -> Result<Peripheral, String> {
    log!("[BLE] Scanning for FLOW 8 ({}s timeout)...", SCAN_TIMEOUT.as_secs());

    adapter
        .start_scan(ScanFilter::default())
        .await
        .map_err(|e| format!("Scan start error: {}", e))?;

    tokio::time::sleep(SCAN_TIMEOUT).await;

    adapter.stop_scan().await.ok();

    let peripherals = adapter
        .peripherals()
        .await
        .map_err(|e| format!("Peripheral list error: {}", e))?;

    log_debug!("[BLE] Found {} device(s) during scan", peripherals.len());

    let mut name_match: Option<Peripheral> = None;
    let target_svc = service_uuid();

    for p in peripherals {
        if let Ok(Some(props)) = p.properties().await {
            let label = props
                .local_name
                .as_deref()
                .unwrap_or("(unnamed)");
            let addr = format!("{:?}", props.address);

            log_debug!("[BLE]   device: \"{}\" addr={}", label, addr);

            if name_match.is_none() {
                if let Some(ref name) = props.local_name {
                    if name.to_uppercase().contains(FLOW8_NAME_KEYWORD) {
                        log!("[BLE] Matched by name: \"{}\"", name);
                        name_match = Some(p.clone());
                        continue;
                    }
                }

                if props.services.contains(&target_svc) {
                    log!("[BLE] Matched by Service UUID: \"{}\" addr={}", label, addr);
                    name_match = Some(p.clone());
                }
            }
        }
    }

    name_match.ok_or_else(|| "FLOW 8 LE not found during scan".to_string())
}

async fn connect_peripheral(peripheral: &Peripheral) -> Result<(), String> {
    log!("[BLE] Connecting ({}s timeout)...", CONNECT_TIMEOUT.as_secs());

    tokio::time::timeout(CONNECT_TIMEOUT, peripheral.connect())
        .await
        .map_err(|_| "Connection timed out".to_string())?
        .map_err(|e| format!("Connection failed: {}", e))?;

    log!("[BLE] Connected. Discovering services...");
    peripheral
        .discover_services()
        .await
        .map_err(|e| format!("Service discovery failed: {}", e))?;

    Ok(())
}

const MAX_CONNECT_RETRIES: u32 = 2;

async fn connect_and_auth(
    peripheral: &Peripheral,
    status_tx: &mpsc::Sender<BleStatus>,
) -> Result<(Characteristic, NotifyStream), String> {
    let mut last_err = String::new();

    for attempt in 0..=MAX_CONNECT_RETRIES {
        if attempt > 0 {
            log_warn!("[BLE] Retry {}/{}...", attempt, MAX_CONNECT_RETRIES);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        let _ = status_tx.send(BleStatus::Connecting);

        match connect_peripheral(peripheral).await {
            Ok(_) => {}
            Err(e) => {
                log_warn!("[BLE] Connect attempt failed: {}", e);
                peripheral.disconnect().await.ok();
                last_err = e;
                continue;
            }
        }

        let target_uuid = characteristic_uuid();
        let characteristic = match peripheral
            .characteristics()
            .into_iter()
            .find(|c| c.uuid == target_uuid)
        {
            Some(c) => c,
            None => {
                last_err = "FLOW 8 BLE characteristic not found".to_string();
                log_warn!("[BLE] {}", last_err);
                peripheral.disconnect().await.ok();
                continue;
            }
        };

        log!("[BLE] Characteristic found.");
        let _ = status_tx.send(BleStatus::Authenticating);

        // subscribe BEFORE auth: the device pushes its 0x37 handshake challenge
        // right after the auth key, and notifications sent before the CCCD is
        // on are lost — without the challenge the session never completes and
        // the device never streams meters
        if let Err(e) = peripheral.subscribe(&characteristic).await {
            log_warn!("[BLE] Subscribe failed: {}", e);
            last_err = format!("Subscribe failed: {}", e);
            peripheral.disconnect().await.ok();
            continue;
        }
        let mut stream = match peripheral.notifications().await {
            Ok(s) => s,
            Err(e) => {
                log_warn!("[BLE] Notification stream error: {}", e);
                last_err = format!("Notification stream error: {}", e);
                peripheral.disconnect().await.ok();
                continue;
            }
        };

        log!("[BLE] Sending auth key...");
        if let Err(e) = peripheral
            .write(&characteristic, &AUTH_PACKET, WriteType::WithResponse)
            .await
        {
            log_warn!("[BLE] Auth write failed: {}", e);
            peripheral.disconnect().await.ok();
            last_err = format!("Auth write failed: {}", e);
            continue;
        }

        tokio::time::sleep(Duration::from_millis(300)).await;

        log!("[BLE] Sending session start...");
        if let Err(e) = peripheral
            .write(&characteristic, &SESSION_START_PACKET, WriteType::WithResponse)
            .await
        {
            log_warn!("[BLE] Session start write failed: {}", e);
            peripheral.disconnect().await.ok();
            last_err = format!("Session start write failed: {}", e);
            continue;
        }

        // the device answers with a 0x37 handshake challenge; reply with the
        // auth key (0x39 + key bytes) to complete the session
        match wait_for_cmd(&mut stream, 0x37, Duration::from_secs(3)).await {
            Some(_) => {
                log!("[BLE] Handshake challenge received, replying with auth key...");
                let reply: Vec<u8> = [0x39].iter().copied().chain(AUTH_PACKET[2..18].iter().copied()).collect();
                let write = peripheral.write(&characteristic, &reply, WriteType::WithoutResponse);
                match tokio::time::timeout(Duration::from_millis(1500), write).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => log_warn!("[BLE] Handshake reply write failed: {}", e),
                    Err(_) => log_debug!("[BLE] Handshake reply: no ack (packet still delivered)"),
                }
                // device should now push the 0x38 state dump and stream meters
                match wait_for_cmd(&mut stream, 0x38, Duration::from_secs(3)).await {
                    Some(_) => log!("[BLE] State dump received — session established"),
                    None => log_warn!("[BLE] No state dump after handshake reply"),
                }
            }
            None => log_warn!("[BLE] No handshake challenge received, proceeding anyway"),
        }

        log!("[BLE] Authenticated and session started");
        return Ok((characteristic, stream));
    }

    Err(last_err)
}

/// Pull notifications until one of the wanted type arrives (payload returned),
/// or the timeout elapses (None). Other notifications are logged and dropped.
async fn wait_for_cmd(stream: &mut NotifyStream, ptype: u8, timeout: Duration) -> Option<Vec<u8>> {
    use tokio_stream::StreamExt;
    tokio::time::timeout(timeout, async {
        while let Some(n) = stream.next().await {
            if n.value.is_empty() {
                continue;
            }
            if n.value[0] == ptype {
                return Some(n.value);
            }
            log_debug!(
                "[BLE] (handshake) notification type=0x{:02X} len={}",
                n.value[0],
                n.value.len()
            );
        }
        None
    })
    .await
    .ok()
    .flatten()
}

fn parse_snapshot_names(data: &[u8]) -> Vec<Option<String>> {
    let mut names: Vec<Option<String>> = Vec::new();

    if data.len() < 3 || data[0] != SNAPSHOT_RESPONSE_TYPE {
        log_warn!(
            "[BLE] Unexpected snapshot response: len={}, first_byte=0x{:02X}",
            data.len(),
            data.first().copied().unwrap_or(0)
        );
        return names;
    }

    let payload = &data[2..data.len().saturating_sub(1)];
    let mut pos = 0;

    log_debug!(
        "[BLE] Parsing snapshot payload: {} bytes (full packet {} bytes)",
        payload.len(),
        data.len()
    );

    while pos < payload.len() && names.len() < 15 {
        let len = payload[pos] as usize;
        pos += 1;

        if len == 0 {
            names.push(None);
        } else if pos + len <= payload.len() {
            match std::str::from_utf8(&payload[pos..pos + len]) {
                Ok(s) => {
                    let trimmed = s.trim();
                    if trimmed.is_empty() {
                        names.push(None);
                    } else {
                        log_debug!("[BLE] Snapshot {}: \"{}\"", names.len() + 1, trimmed);
                        names.push(Some(trimmed.to_string()));
                    }
                }
                Err(_) => {
                    log_debug!(
                        "[BLE] Snapshot {}: invalid UTF-8 ({} bytes)",
                        names.len() + 1,
                        len
                    );
                    names.push(None);
                }
            }
            pos += len;
        } else {
            log_debug!("[BLE] Snapshot parse ended: pos={} + len={} > payload.len()={}", pos - 1, len, payload.len());
            break;
        }
    }

    names
}

const SETTING_READ_IDS: [u8; 12] = [
    0x01, 0x02, 0x03, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
];

/// Probe GetSetting (0x26) for each known id while the notification stream is
/// idle (between the state-dump drain and the snapshot request). The response
/// shape is unverified — every notification is logged; values are parsed from
/// the best-matching shapes so one device test settles it.
async fn read_device_settings(
    peripheral: &Peripheral,
    characteristic: &Characteristic,
    stream: &mut NotifyStream,
) -> Vec<(u8, u8)> {
    use tokio_stream::StreamExt;

    log!("[BLE] Reading device settings (0x26)...");
    let mut found: Vec<(u8, u8)> = Vec::new();
    for &id in &SETTING_READ_IDS {
        let packet = [0x26u8, id];
        let write = peripheral.write(
            characteristic,
            &packet,
            WriteType::WithoutResponse,
        );
        // cap the ack wait; without-response writes never complete on BlueZ
        // but the packet is delivered
        let _ = tokio::time::timeout(Duration::from_millis(300), write).await;
        while let Ok(Some(n)) =
            tokio::time::timeout(Duration::from_millis(120), stream.next()).await
        {
            let v = &n.value;
            log_debug!("[BLE] Settings readback: {:02X?}", &v[..v.len().min(12)]);
            if v.len() >= 3 && v[0] == 0x26 {
                found.push((v[1], v[2]));
            } else if v.len() == 2 && SETTING_READ_IDS.contains(&v[0]) {
                found.push((v[0], v[1]));
            }
        }
    }
    log!("[BLE] Settings readback: {} values parsed", found.len());
    found
}

async fn fetch_snapshot_names(
    peripheral: &Peripheral,
    characteristic: &Characteristic,
    meter_tx: Option<mpsc::Sender<Vec<f32>>>,
    settings_tx: Option<mpsc::Sender<(u8, u8)>>,
    mut notification_stream: NotifyStream,
) -> Result<Vec<Option<String>>, String> {
    use tokio_stream::StreamExt;

    log!("[BLE] Draining state dump (0x38)...");
    let drain_result = tokio::time::timeout(Duration::from_secs(3), async {
        let mut dump_chunks = 0u32;
        while let Some(notification) = notification_stream.next().await {
            if !notification.value.is_empty() {
                let ptype = notification.value[0];
                log_debug!(
                    "[BLE] Notification: type=0x{:02X}, len={}",
                    ptype,
                    notification.value.len()
                );
                if ptype == STATE_DUMP_TYPE {
                    dump_chunks += 1;
                    if dump_chunks >= 4 {
                        return dump_chunks;
                    }
                }
            }
        }
        dump_chunks
    })
    .await;

    match drain_result {
        Ok(n) => log!("[BLE] State dump received ({} chunks)", n),
        Err(_) => log_warn!("[BLE] State dump wait timed out, proceeding anyway"),
    }

    // one-shot settings readback while the stream is idle
    if let Some(settings_tx) = settings_tx {
        for (id, value) in
            read_device_settings(peripheral, characteristic, &mut notification_stream).await
        {
            let _ = settings_tx.send((id, value));
        }
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    log!("[BLE] Requesting snapshot names (0x07)...");
    if let Err(e) = peripheral
        .write(characteristic, &CONFIG_REQUEST_PACKET, WriteType::WithResponse)
        .await
    {
        log_warn!("[BLE] Config request write failed: {}", e);
        return Ok(Vec::new());
    }

    let result = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(notification) = notification_stream.next().await {
            if !notification.value.is_empty() {
                let ptype = notification.value[0];
                log_debug!(
                    "[BLE] Notification: type=0x{:02X}, len={}",
                    ptype,
                    notification.value.len()
                );
                if ptype == SNAPSHOT_RESPONSE_TYPE {
                    return Some(parse_snapshot_names(&notification.value));
                }
            }
        }
        None
    })
    .await;

    // the subscription stays alive for the session: hand the open notification
    // stream to the meter loop (a second subscribe()/notifications() call
    // stalls on BlueZ, so it must reuse this one)
    if let Some(meter_tx) = meter_tx {
        start_meter_loop(
            peripheral.clone(),
            characteristic.clone(),
            notification_stream,
            meter_tx,
        );
    }

    // keep the subscription alive for the session (the meter stream shares it)
    match result {
        Ok(Some(names)) => {
            let named_count = names.iter().filter(|n| n.is_some()).count();
            log!(
                "[BLE] Parsed {} snapshot slots ({} with names)",
                names.len(),
                named_count
            );
            Ok(names)
        }
        Ok(None) => {
            log_warn!("[BLE] Notification stream ended without snapshot response");
            Ok(Vec::new())
        }
        Err(_) => {
            log_warn!("[BLE] Timeout waiting for snapshot names (5s)");
            Ok(Vec::new())
        }
    }
}

pub fn connect_flow8_ble(
    status_tx: mpsc::Sender<BleStatus>,
    snapshot_tx: mpsc::Sender<Vec<Option<String>>>,
    meter_tx: mpsc::Sender<Vec<f32>>,
    settings_tx: mpsc::Sender<(u8, u8)>,
) -> Result<BleConnection, String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("Tokio runtime error: {}", e))?;

    rt.block_on(async {
        let _ = status_tx.send(BleStatus::Scanning);
        let adapter = get_adapter().await?;

        let peripheral = match scan_for_flow8(&adapter).await {
            Ok(p) => p,
            Err(e) => {
                let _ = status_tx.send(BleStatus::Error);
                return Err(e);
            }
        };

        let (characteristic, notification_stream) = match connect_and_auth(&peripheral, &status_tx).await {
            Ok(c) => c,
            Err(e) => {
                let _ = status_tx.send(BleStatus::Error);
                return Err(e);
            }
        };

        let _ = status_tx.send(BleStatus::Connected);

        match fetch_snapshot_names(
            &peripheral,
            &characteristic,
            Some(meter_tx),
            Some(settings_tx),
            notification_stream,
        )
        .await
        {
            Ok(names) if !names.is_empty() => {
                let _ = snapshot_tx.send(names);
            }
            Ok(_) => log_warn!("[BLE] No snapshot names received"),
            Err(e) => log_warn!("[BLE] Failed to get snapshot names: {}", e),
        }

        Ok(BleConnection {
            peripheral,
            characteristic,
        })
    })
}

pub fn send_dump_trigger(conn: &BleConnection) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("Tokio runtime error: {}", e))?;

    rt.block_on(async {
        log!("[BLE] Sending dump trigger (0x4B)...");
        // the device never completes the D-Bus WriteValue call for this packet,
        // but delivers it on air (~250 ms) — cap the wait, the dump arrives via USB
        let write = conn.peripheral.write(
            &conn.characteristic,
            &DUMP_TRIGGER_PACKET,
            WriteType::WithoutResponse,
        );
        match tokio::time::timeout(Duration::from_millis(1500), write).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(format!("Dump trigger write failed: {}", e)),
            Err(_) => log!("[BLE] Dump trigger sent (no ack from device)"),
        }

        log!("[BLE] Dump trigger sent. SysEx should arrive via USB MIDI.");
        Ok(())
    })
}

// ── BLE write commands (opcodes from the official FLOW Mix app protocol) ──
// Wire format: [opcode][payload...] in a single GATT write. Channel rename (0x05)
// verified working on-device 2026-11-03; snapshot save/rename derived from the native
// cmd descriptor table — if the device ignores one, try a variant (icon bytes, length prefix).

const CMD_CHANNEL_LABEL: u8 = 0x05; // [uid][icon u16][name]
const CMD_SNAPSHOT_SAVE: u8 = 0x19; // [slot][name]
const CMD_SNAPSHOT_RENAME: u8 = 0x32; // [slot][name]
const CHANNEL_NAME_MAX: usize = 8; // panel labels are 8 chars

pub fn send_ble_command(conn: &BleConnection, opcode: u8, payload: &[u8]) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("Tokio runtime error: {}", e))?;
    rt.block_on(async {
        let mut packet = Vec::with_capacity(payload.len() + 1);
        packet.push(opcode);
        packet.extend_from_slice(payload);
        let write = conn.peripheral.write(
            &conn.characteristic,
            &packet,
            // device answers via notifications/USB, not with a write response
            WriteType::WithoutResponse,
        );
        // device never completes the D-Bus WriteValue call; cap the ack wait —
        // the packet itself is delivered (verify via panel/dump)
        match tokio::time::timeout(Duration::from_millis(1500), write).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(format!("BLE command 0x{:02X} failed: {}", opcode, e)),
            Err(_) => Ok(()),
        }
    })
}

/// Rename a channel: ChannelLabel (op 0x05), [uid][icon u16 = 0][ascii name].
/// uid = channel id (0..=6, same order as the state dump).
pub fn send_channel_rename(conn: &BleConnection, uid: u8, name: &str) -> Result<(), String> {
    let mut name_buf: Vec<u8> = name
        .chars()
        .take(CHANNEL_NAME_MAX)
        .map(|c| c as u8)
        .collect();
    name_buf.truncate(CHANNEL_NAME_MAX);
    let mut payload = vec![uid, 0x00, 0x00];
    payload.append(&mut name_buf);
    send_ble_command(conn, CMD_CHANNEL_LABEL, &payload)
}

/// Rename a snapshot on the device: SnapshotRename (op 0x32), [slot][name].
pub fn send_snapshot_rename(conn: &BleConnection, slot: u8, name: &str) -> Result<(), String> {
    let mut payload = vec![slot];
    payload.extend_from_slice(name.as_bytes());
    send_ble_command(conn, CMD_SNAPSHOT_RENAME, &payload)
}

/// Save the current mixer state into a device snapshot slot: SnapshotSave (op 0x19), [slot][name].
pub fn send_snapshot_save(conn: &BleConnection, slot: u8, name: &str) -> Result<(), String> {
    let mut payload = vec![slot];
    payload.extend_from_slice(name.as_bytes());
    send_ble_command(conn, CMD_SNAPSHOT_SAVE, &payload)
}

/// Delete a device snapshot slot: SnapshotDelete (op 0x12), [slot].
pub fn send_snapshot_delete(conn: &BleConnection, slot: u8) -> Result<(), String> {
    send_ble_command(conn, 0x12, &[slot])
}

/// Recall a snapshot over BLE: SnapshotLoad (op 0x20), [slot].
/// Works even when the USB MIDI path (Program Change ch 16) is wedged.
pub fn send_snapshot_load(conn: &BleConnection, slot: u8) -> Result<(), String> {
    send_ble_command(conn, 0x20, &[slot])
}

/// FX tempo: official app computes a 5-tap rolling average, then sends op 0x40 [u16 bpm LE].
pub fn send_fx_tempo(conn: &BleConnection, bpm: u16) -> Result<(), String> {
    send_ble_command(conn, 0x40, &[bpm as u8, (bpm >> 8) as u8])
}

/// Device setting write: Setting (op 0x25), [id][value].
/// shortcut: packet form unverified on-device; if ignored, try variants.
pub fn send_setting(conn: &BleConnection, id: u8, value: u8) -> Result<(), String> {
    send_ble_command(conn, 0x25, &[id, value])
}

/// Input preset load: InputChannelState (op 0x17). Official layout
/// (loadInputChannelPreset @0xb66f4 + setChannelState @0xfb284 in the native
/// lib): cmd[0]=0x17, cmd[24]=ch_id, then the input-channel state —
/// flags at +1..4 (stereo, mute, phase, phantom), gain f32 at +12,
/// compressor f32 at +16, HPF Hz u16 at +20, pan f32 at +24,
/// icon u16 at +70, channel label (%.20s) at +72.
pub fn send_input_preset(
    conn: &BleConnection,
    ch_id: u8,
    preset: &crate::model::presets::InputPreset,
) -> Result<(), String> {
    let mut payload = vec![0u8; 23 + 100];
    payload[23] = ch_id; // cmd[24]
    payload[24] = 0x01; // state+1: stereo flag (official presets set 1)
                        // state+2/+3 (mute, phase) stay 0
    payload[27] = preset.phantom as u8; // state+4
    payload[23 + 12..23 + 16].copy_from_slice(&(preset.gain_db as f32).to_le_bytes());
    payload[23 + 20..23 + 22].copy_from_slice(&preset.hpf_hz.to_le_bytes());
    // pan f32 at state+24 stays 0.0 (center); sends f32s at state+28..44 = 0
    let label: Vec<u8> = preset.name.bytes().take(20).collect(); // %.20s
    payload[23 + 72..23 + 72 + label.len()].copy_from_slice(&label);
    send_ble_command(conn, 0x17, &payload)
}

/// FX setup block: FxSetup (op 0x11). Official layout (setFxSetup @0xaf270 +
/// updateFXOutputRouting @0xb9c34): cmd[24]=fx_ch_id, cmd[25..27] = the FX
/// channel's first/second/third param values, cmd[28..30] = route Main/Mon1/Mon2.
/// Used for routing checkboxes AND param sends (updateFXPresetValue builds the
/// same block for a single param change).
pub fn send_fx_setup(
    conn: &BleConnection,
    fx_id: u8,
    p1: u8,
    p2: u8,
    p3: u8,
    to_main: bool,
    to_mon1: bool,
    to_mon2: bool,
) -> Result<(), String> {
    let mut payload = vec![0u8; 30];
    payload[23] = fx_id; // cmd[24]
    payload[24] = p1;
    payload[25] = p2;
    payload[26] = p3;
    payload[27] = to_main as u8; // cmd[28]
    payload[28] = to_mon1 as u8;
    payload[29] = to_mon2 as u8;
    send_ble_command(conn, 0x11, &payload)
}

/// FX preset load: FXLoadPreset (op 0x31). queueFXLoadPresetCommandPackets
/// @0x7abec: cmd[24]=fx_ch_id, cmd[25]=preset (1-based — the dump reports
/// preset_raw 1..=16). `preset` here is the 1-based wire value.
pub fn send_fx_preset(conn: &BleConnection, fx_id: u8, preset: u8) -> Result<(), String> {
    let mut payload = vec![0u8; 25];
    payload[23] = fx_id; // cmd[24]
    payload[24] = preset; // cmd[25]
    send_ble_command(conn, 0x31, &payload)
}

// ── VU meters: MeterRequest (0x21); device streams MeterUpdate (0x22) ──
// Official layout (setMeterOutputChannelId @0xb1564 in the native lib):
// count byte + 10 channel ids copied from METER_REQUEST_CHANNELS
// (40 41 42 43 C4 C5 C6 8F + 2 zero pad); the app's wire serializer moves
// the type byte into the 6-byte chunk header, so the payload has NO type.
const METER_IDS: [u8; 10] = [0x40, 0x41, 0x42, 0x43, 0xC4, 0xC5, 0xC6, 0x8F, 0x00, 0x00];
const METER_UPDATE_TYPE: u8 = 0x22;

/// piecewise dB → linear 0..1 (−70 dB floor → 0, 0 dBFS → 1.0), from the official app
pub fn meter_db_to_linear(db: f32) -> f32 {
    let v = if db < -70.0 {
        0.0
    } else if db < -60.0 {
        0.25 * (db + 70.0)
    } else if db < -50.0 {
        2.5 + 0.5 * (db + 60.0)
    } else if db < -40.0 {
        7.5 + 0.75 * (db + 50.0)
    } else if db < -30.0 {
        15.0 + 1.5 * (db + 40.0)
    } else if db < -20.0 {
        30.0 + 2.0 * (db + 30.0)
    } else if db < 0.0 {
        50.0 + 2.5 * (db + 20.0)
    } else {
        100.0
    };
    v / 100.0
}

/// Run the meter receive loop on a thread, using the notification stream that
/// fetch_snapshot_names already opened (a second subscribe()/notifications()
/// call stalls on BlueZ). Sends MeterRequest (0x21), then parses
/// MeterUpdate (0x22) notifications into 15 raw dB floats; re-requests
/// after 5 s of silence.
pub fn start_meter_loop(
    peripheral: Peripheral,
    characteristic: Characteristic,
    notifications: NotifyStream,
    meter_tx: mpsc::Sender<Vec<f32>>,
) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(async move {
            use tokio_stream::StreamExt;

            let mut notifications = notifications;
            let mut first_data = true;
            let mut other_logged = 0u32;
            let mut request_variant = 0u32;
            log!("[BLE] Meter loop running (shared notification stream)");
            send_meter_request(&peripheral, &characteristic, request_variant).await;

            // 5 s of silence → re-request with the next variant (device may
            // have stopped pushing, or the request shape is wrong)
            loop {
                let notification =
                    match tokio::time::timeout(Duration::from_secs(5), notifications.next()).await
                    {
                        Ok(Some(n)) => n,
                        Ok(None) => {
                            log_warn!("[BLE] Meter loop: notification stream ended");
                            break;
                        }
                        Err(_) => {
                            request_variant += 1;
                            send_meter_request(&peripheral, &characteristic, request_variant).await;
                            continue;
                        }
                    };
                if notification.value.is_empty() || notification.value[0] != METER_UPDATE_TYPE {
                    if other_logged < 3 {
                        other_logged += 1;
                        log_debug!(
                            "[BLE] Meter loop: non-meter notification type=0x{:02X} len={}",
                            notification.value.first().copied().unwrap_or(0),
                            notification.value.len()
                        );
                    }
                    continue;
                }
                // payload = [0x22][15 × f32 LE dB][u16 GR at +60]
                let v = &notification.value;
                let mut values = Vec::with_capacity(15);
                for i in 0..15 {
                    let off = 1 + i * 4;
                    if off + 4 <= v.len() {
                        values.push(f32::from_le_bytes([
                            v[off],
                            v[off + 1],
                            v[off + 2],
                            v[off + 3],
                        ]));
                    }
                }
                if first_data {
                    first_data = false;
                    let floats: Vec<String> =
                        values.iter().map(|f| format!("{:.1}", f)).collect();
                    log!(
                        "[BLE] Meter data flowing ({} floats): {}",
                        values.len(),
                        floats.join(" ")
                    );
                }
                let _ = meter_tx.send(values);
            }
        });
    });
}

async fn send_meter_request(peripheral: &Peripheral, characteristic: &Characteristic, variant: u32) {
    // Official MeterRequest payload = [count=8][10 channel ids] (count at
    // struct[24], ids at struct[25..35] in the native lib). The device
    // accepts bare writes when sizes match (the 0x07 names request proves
    // it), so cycle bare / 6-byte-header-framed / full-struct and let the
    // 0x22 stream tell us which shape it takes.
    let mut payload = Vec::with_capacity(11);
    payload.push(0x08); // meter channel count (output selected → 8)
    payload.extend_from_slice(&METER_IDS);
    let framed: Vec<u8> = {
        // wire header [len u16 LE][type][chunk_idx][chunk_count][seq]
        let mut p = vec![payload.len() as u8, 0, 0x21, 0, 1, 0];
        p.extend_from_slice(&payload);
        p
    };
    // [0x21] + 23 zero bytes (struct bytes 1..24) + payload — if the device's
    // bare parser copies into struct bytes 1.., count lands at struct[24]
    let mut bare_struct = vec![0x21u8; 24];
    bare_struct.extend_from_slice(&payload);
    let (packet, mode) = match variant % 3 {
        0 => ([vec![0x21u8], payload.clone()].concat(), "bare"),
        1 => (framed, "framed"),
        _ => (bare_struct, "bare-struct"),
    };
    log_debug!(
        "[BLE] Meter request variant {} ({}, {} bytes)",
        variant % 3,
        mode,
        packet.len()
    );
    let write = peripheral.write(characteristic, &packet, WriteType::WithoutResponse);
    match tokio::time::timeout(Duration::from_millis(1500), write).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => log_debug!("[BLE] Meter request write error: {}", e),
        Err(_) => log_debug!("[BLE] Meter request: no ack (packet still delivered)"),
    }
}

/// Re-fetch the snapshot list from the device. One-shot: opens its own
/// notification stream (btleplug broadcasts to every stream) so the meter
/// loop's stream is untouched.
pub fn refresh_snapshot_names(conn: &BleConnection) -> Vec<Option<String>> {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            log_error!("[BLE] Could not create runtime for snapshot refresh: {}", e);
            return Vec::new();
        }
    };
    rt.block_on(async {
        // no subscribe() here: the session-level subscription from connect is
        // already active and a second one stalls on BlueZ (~30 s D-Bus timeout)
        let stream = match conn.peripheral.notifications().await {
            Ok(s) => s,
            Err(e) => {
                log_warn!("[BLE] Snapshot refresh: notification stream error: {}", e);
                return Vec::new();
            }
        };
        match fetch_snapshot_names(&conn.peripheral, &conn.characteristic, None, None, stream).await {
            Ok(names) => names,
            Err(e) => {
                log_warn!("[BLE] Snapshot refresh failed: {}", e);
                Vec::new()
            }
        }
    })
}

pub fn disconnect(conn: &BleConnection) {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            log_error!("[BLE] Could not create runtime for disconnect: {}", e);
            return;
        }
    };

    rt.block_on(async {
        if let Err(e) = conn.peripheral.disconnect().await {
            log_warn!("[BLE] Disconnect error: {}", e);
        } else {
            log!("[BLE] Disconnected");
        }
    });
}
