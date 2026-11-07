use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use std::pin::Pin;
use std::sync::mpsc;
use std::time::Duration;
use uuid::Uuid;

use crate::model::flow8::MeterMsg;
use crate::{log, log_debug, log_error, log_warn};

const SERVICE_UUID: &str = "14839ad4-8d7e-415c-9a42-167340cf2339";
const CHARACTERISTIC_UUID: &str = "0034594a-a8e7-4b1a-a6b1-cd5243059a57";
const FLOW8_NAME_KEYWORD: &str = "FLOW";
const FLOW8_NAME_EXACT: &str = "FLOW 8 LE";
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

    // the mixer advertises twice: the real GATT target "FLOW 8 LE" and a
    // name-only "FLOW 8 Audio (...)" entry with no services (connecting to
    // it fails with br-connection-key-missing). Exact name wins, then the
    // main session's address, then keyword/service-UUID as fallback.
    let mut exact: Option<Peripheral> = None;
    let mut fuzzy: Option<Peripheral> = None;
    let target_svc = service_uuid();

    for p in peripherals {
        if let Ok(Some(props)) = p.properties().await {
            let label = props
                .local_name
                .as_deref()
                .unwrap_or("(unnamed)");
            let addr = format!("{:?}", props.address);

            log_debug!("[BLE]   device: \"{}\" addr={}", label, addr);

            if exact.is_none() {
                if props.local_name.as_deref() == Some(FLOW8_NAME_EXACT) {
                    log!("[BLE] Matched exact name: \"{}\" addr={}", label, addr);
                    exact = Some(p.clone());
                    continue;
                }
            }

            if fuzzy.is_none() {
                if let Some(ref name) = props.local_name {
                    if name.to_uppercase().contains(FLOW8_NAME_KEYWORD) {
                        log!("[BLE] Fuzzy name match: \"{}\" addr={}", name, addr);
                        fuzzy = Some(p.clone());
                        continue;
                    }
                }

                if props.services.contains(&target_svc) {
                    log!("[BLE] Matched by Service UUID: \"{}\" addr={}", label, addr);
                    fuzzy = Some(p.clone());
                }
            }
        }
    }

    exact
        .or(fuzzy)
        .ok_or_else(|| "FLOW 8 LE not found during scan".to_string())
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

const MAX_CONNECT_RETRIES: u32 = 3; // each attempt waits 30 s for the challenge

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

        // subscribe BEFORE the handshake: the mixer's 0x37 challenge arrives
        // unsolicited right after connect, and notifications sent before the
        // CCCD is on are lost
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

        // The mixer initiates the handshake: it streams a 0x35 challenge
        // every 500 ms (dispatch arm 0xB1060 → onHandShakeCommandReceived,
        // which queues the 0x39+key reply). We reply 0x39+key, the mixer
        // confirms with 0x36 (onHandShakeReplyCommandReceived), we send the
        // 0x37 session-start, and the mixer follows with the 0x38 state dump.
        // 0x37 has NO RX arm in the dispatch table — waiting for it was wrong.
        log!("[BLE] Waiting for handshake challenge (0x35)...");
        match wait_for_cmd(&mut stream, 0x35, Duration::from_secs(30)).await {
            Some(_) => {
                log!("[BLE] Handshake challenge (0x35) received, replying with auth key...");
                // the full on-air auth packet: [0x39][0x01][16-byte key][cksum]
                // (proven 2026-11-04 — the trimmed [0x39]+key reply is ignored)
                let write = peripheral.write(&characteristic, &AUTH_PACKET, WriteType::WithoutResponse);
                match tokio::time::timeout(Duration::from_millis(1500), write).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => log_warn!("[BLE] Handshake reply write failed: {}", e),
                    Err(_) => log_debug!("[BLE] Handshake reply: no ack (packet still delivered)"),
                }
                // mixer confirms with the 0x36 handshake reply
                match wait_for_cmd(&mut stream, 0x36, Duration::from_secs(5)).await {
                    Some(_) => {
                        log!("[BLE] Handshake reply (0x36) — sending session start...");
                        if let Err(e) = peripheral
                            .write(&characteristic, &SESSION_START_PACKET, WriteType::WithResponse)
                            .await
                        {
                            log_warn!("[BLE] Session start write failed: {}", e);
                        }
                        match wait_for_cmd(&mut stream, 0x38, Duration::from_secs(5)).await {
                            Some(_) => {
                                log!("[BLE] State dump (0x38) received — session established")
                            }
                            None => log_warn!("[BLE] No state dump (0x38) after session start"),
                        }
                        log!("[BLE] Authenticated and session started");
                        return Ok((characteristic, stream));
                    }
                    None => {
                        last_err = "No handshake reply (0x36) after auth".to_string();
                        log_warn!("[BLE] {}", last_err);
                        peripheral.disconnect().await.ok();
                        continue;
                    }
                }
            }
            None => {
                // 30 s window covers a PAIR REMOTE press; the mixer streams
                // its 0x35 challenge every 500 ms once it has no remote
                log_warn!(
                    "[BLE] No handshake challenge in 30 s (retry {}/{}) — press MENU → PAIRING → PAIR REMOTE",
                    attempt + 1,
                    MAX_CONNECT_RETRIES
                );
                peripheral.disconnect().await.ok();
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
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
    meter_tx: Option<mpsc::Sender<crate::model::flow8::MeterMsg>>,
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

    // the meter loop rides THIS session's connection: it is authenticated
    // (the handshake is per-connection and the mixer answers meter polls only
    // on an authenticated one — the old separate ride-along session never
    // handshook and its polls went unanswered)
    if let Some(meter_tx) = meter_tx {
        start_meter_loop(peripheral.clone(), characteristic.clone(), meter_tx);
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
    meter_tx: mpsc::Sender<crate::model::flow8::MeterMsg>,
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

/// One GATT write, capped: this device never completes the D-Bus WriteValue
/// call — the packet is on air in ~250 ms regardless.
async fn write_packet(peripheral: &Peripheral, characteristic: &Characteristic, packet: &[u8], opcode: u8) {
    let write = peripheral.write(characteristic, packet, WriteType::WithoutResponse);
    match tokio::time::timeout(Duration::from_millis(300), write).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => log_debug!("[BLE] cmd 0x{:02X} write error: {}", opcode, e),
        Err(_) => {}
    }
}

/// Compact commands (0x05 rename, 0x07 names, 0x12/0x20 snapshot, 0x19/0x32
/// save/rename, 0x25 setting, 0x40 tempo): fields live at struct[1..],
/// so the bare form is just [opcode][fields].
pub fn send_ble_command(conn: &BleConnection, opcode: u8, payload: &[u8]) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("Tokio runtime error: {}", e))?;
    rt.block_on(async {
        let mut packet = Vec::with_capacity(payload.len() + 1);
        packet.push(opcode);
        packet.extend_from_slice(payload);
        write_packet(&conn.peripheral, &conn.characteristic, &packet, opcode).await;
        Ok(())
    })
}

/// Struct commands (0x11 FX setup, 0x17 input preset, 0x21 meter request,
/// 0x31 FX preset, 0x41 select output) read their fields at struct[24..]:
/// the compact bare form is silently dropped (data lands at struct[1]).
/// Send the padded bare form AND the official chunked form
/// ([len u16][type][idx][count][seq] + ≤32-byte slices of the same fields).
pub fn send_ble_struct_command(conn: &BleConnection, opcode: u8, payload: &[u8]) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("Tokio runtime error: {}", e))?;
    rt.block_on(async {
        write_struct_command(&conn.peripheral, &conn.characteristic, opcode, payload).await;
        Ok(())
    })
}

async fn write_struct_command(peripheral: &Peripheral, characteristic: &Characteristic, opcode: u8, payload: &[u8]) {
    static SEQ: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

    // bare struct form fits in a default-MTU (23) write → send it; anything
    // larger is truncated by BlueZ and must go chunked only
    if 24 + payload.len() <= 20 {
        let mut bare = vec![0u8; 24 + payload.len()];
        bare[0] = opcode;
        bare[24..].copy_from_slice(payload);
        write_packet(peripheral, characteristic, &bare, opcode).await;
    }

    // chunked form: fields = struct bytes 1.. (the zero pad included)
    let mut fields = vec![0u8; 23 + payload.len()];
    fields[23..].copy_from_slice(payload);
    // 17 data bytes + 6-byte header = 23 = smallest MTU — BlueZ silently
    // truncates write-without-response beyond MTU-3 (every command that
    // ever worked was ≤20 bytes; every struct command was 33+ and died)
    const CHUNK: usize = 17;
    let count = fields.len().div_ceil(CHUNK);
    let seq = if count > 1 {
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    } else {
        0 // single-chunk commands always carry seq 0 (native lib @0xec5c4)
    };
    for (idx, chunk) in fields.chunks(CHUNK).enumerate() {
        let mut p = Vec::with_capacity(chunk.len() + 7);
        p.push(chunk.len() as u8);
        p.push(0);
        p.push(opcode);
        p.push(idx as u8);
        p.push(count as u8);
        p.push(seq);
        p.extend_from_slice(chunk);
        write_packet(peripheral, characteristic, &p, opcode).await;
    }
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
    // fields only — send_ble_struct_command prepends the 23-zero struct pad
    let mut state = vec![0u8; 100];
    state[0] = ch_id;
    state[1] = 0x01; // stereo flag (official presets set 1)
                     // state+2/+3 (mute, phase) stay 0
    state[4] = preset.phantom as u8;
    state[12..16].copy_from_slice(&(preset.gain_db as f32).to_le_bytes());
    state[20..22].copy_from_slice(&preset.hpf_hz.to_le_bytes());
    // pan f32 at state+24 stays 0.0 (center); sends f32s at state+28..44 = 0
    let label: Vec<u8> = preset.name.bytes().take(20).collect(); // %.20s
    state[72..72 + label.len()].copy_from_slice(&label);
    send_ble_struct_command(conn, 0x17, &state)
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
    let payload = [
        fx_id, p1, p2, p3,
        to_main as u8,
        to_mon1 as u8,
        to_mon2 as u8,
    ]; // fields land at cmd[24..30] — the writer prepends the struct pad
    send_ble_struct_command(conn, 0x11, &payload)
}

/// FX preset load: FXLoadPreset (op 0x31). queueFXLoadPresetCommandPackets
/// @0x7abec: cmd[24]=fx_ch_id, cmd[25]=preset (1-based — the dump reports
/// preset_raw 1..=16). `preset` here is the 1-based wire value.
pub fn send_fx_preset(conn: &BleConnection, fx_id: u8, preset: u8) -> Result<(), String> {
    send_ble_struct_command(conn, 0x31, &[fx_id, preset])
}

// ── VU meters: MeterRequest (0x21); device streams MeterUpdate (0x22) ──
const METER_UPDATE_TYPE: u8 = 0x22;
// setMeterOutputChannelId @0xb1564: cmd[24]=count(8), cmd[25..31] = input ids
// (id|0x40 mono, |0xC0 stereo), cmd[32] = the SELECTED output (id|0xC0) — the
// device reports that output's L/R in update floats 10/11 (the official app
// patches METER_INDEX_TO_CHANNEL_INDEX[10]/[11] with the output's channel
// index). SelectOutputChannel (0x41, [output id]) switches the device side.
const METER_INPUT_IDS: [u8; 7] = [0x40, 0x41, 0x42, 0x43, 0xC4, 0xC5, 0xC6];
/// metered outputs in rotation: Main (7), Mon 1 (8), Mon 2 (9) — bus id | 0xC0 (stereo)
const METER_OUTPUT_BYTES: [u8; 3] = [0xC7, 0xC8, 0xC9];

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

/// Run the meter receive loop on its own thread, riding the main session's
/// authenticated connection (a second subscribe()/notifications() call on the
/// same connection is fine — the CCCD is per-connection and the mixer answers
/// meter polls only on an authenticated one).
pub fn start_meter_loop(
    peripheral: Peripheral,
    characteristic: Characteristic,
    meter_tx: mpsc::Sender<MeterMsg>,
) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(async move {
            use tokio_stream::StreamExt;

            let mut notifications = loop {
                match peripheral.subscribe(&characteristic).await {
                    Ok(_) => {}
                    Err(e) => {
                        log_warn!("[BLE] Meter loop: subscribe failed: {} — retrying in 10 s", e);
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                }
                match peripheral.notifications().await {
                    Ok(s) => break s,
                    Err(e) => {
                        log_warn!(
                            "[BLE] Meter loop: notification stream failed: {} — retrying in 10 s",
                            e
                        );
                        tokio::time::sleep(Duration::from_secs(10)).await;
                    }
                }
            };
            let mut first_data = true;
            let mut other_logged = 0u32;
            let mut samples_logged = 0u32;
            let mut rotation = 0usize;
            log!("[BLE] Meter loop running (main session stream)");
            // the device answers MeterRequest with level pushes (a pull
            // model): the official app polls at meter rate (~10 Hz). One
            // request after pairing produced a single 3-record burst, then
            // silence — so poll every 100 ms and rotate the metered output
            // (Main → Mon 1 → Mon 2) every 2 s.
            write_struct_command(&peripheral, &characteristic, 0x41, &[7]).await;
            send_meter_poll(&peripheral, &characteristic, 0).await;
            let mut last_sent = std::time::Instant::now();
            loop {
                let wait = Duration::from_millis(100).saturating_sub(last_sent.elapsed());
                let notification = match tokio::time::timeout(wait, notifications.next()).await {
                    Ok(Some(n)) => Some(n),
                    Ok(None) => {
                        log_warn!("[BLE] Meter loop: notification stream ended");
                        break;
                    }
                    Err(_) => None,
                };
                if let Some(notification) = notification {
                    let v = &notification.value;
                    if v.first() == Some(&METER_UPDATE_TYPE) {
                        // payload = [0x22][15 × f32 LE dB][u16 packed GR at +60]
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
                        let _ = meter_tx.send(MeterMsg::Frame(
                            values,
                            rotation % METER_OUTPUT_BYTES.len(),
                        ));
                    } else if v.len() == 6
                        && v[0] == 0x06
                        && v[..5]
                            .iter()
                            .fold(0u8, |a, b| a.wrapping_add(*b))
                            == v[5]
                    {
                        // meter sample record: [06][01][in_id][out_id][value][cksum],
                        // checksum = byte-sum of the first 5 bytes (verified on
                        // all captures). Type 0x06 = ChannelMaster push: a fader
                        // position (queueChannelMaster: getChannelById(cmd[25]),
                        // setVolumedBForInputChannelId(cmd[24], f32)) — arrives
                        // when our MIDI-driven moves land or a physical fader
                        // moves. Feeds fader sync, not the meters.
                        let _ = meter_tx.send(MeterMsg::Fader {
                            input: v[2],
                            output: v[3],
                            value: v[4],
                        });
                        if samples_logged < 60 {
                            samples_logged += 1;
                            log_debug!(
                                "[BLE] Fader push: in=0x{:02X} out=0x{:02X} val=0x{:02X}",
                                v[2],
                                v[3],
                                v[4]
                            );
                        }
                    } else if v.len() == 5 && (v[0] == 0x01 || v[0] == 0x08) {
                        // mute/solo state echo from the device — handled locally
                    } else {
                        // hex-log the first unknowns: anything else that flows
                        // on a live session reveals itself here
                        if other_logged < 100 {
                            other_logged += 1;
                            let hex: Vec<String> =
                                v.iter().map(|b| format!("{:02X}", b)).collect();
                            log_debug!(
                                "[BLE] Meter loop: other notification len={} {}",
                                v.len(),
                                hex.join(" ")
                            );
                        }
                    }
                }
                if last_sent.elapsed() >= Duration::from_secs(2) {
                    rotation += 1;
                    let out_id = 7 + (rotation % METER_OUTPUT_BYTES.len()) as u8;
                    log_debug!("[BLE] Meter rotation → output {}", out_id);
                    write_struct_command(&peripheral, &characteristic, 0x41, &[out_id]).await;
                    last_sent = std::time::Instant::now();
                } else {
                    // poll: request levels for the currently selected output
                    let rot = rotation % METER_OUTPUT_BYTES.len();
                    send_meter_poll(&peripheral, &characteristic, rot).await;
                }
            }
        });
    });
}

/// MeterRequest poll (0x21): fields = count=8 + the full 10-slot id table
/// (40 41 42 43 C4 C5 C6 <out> 00 00) — the selected output overwrites slot
/// 7, the memcpy in setMeterOutputChannelId @0xb1564 always copies 10 bytes.
async fn send_meter_poll(peripheral: &Peripheral, characteristic: &Characteristic, rot: usize) {
    let out_byte = METER_OUTPUT_BYTES[rot];
    let fields = [
        0x08u8, // meter channel count (7 inputs + 1 output)
        METER_INPUT_IDS[0],
        METER_INPUT_IDS[1],
        METER_INPUT_IDS[2],
        METER_INPUT_IDS[3],
        METER_INPUT_IDS[4],
        METER_INPUT_IDS[5],
        METER_INPUT_IDS[6],
        out_byte,
        0x00,
        0x00,
    ];
    write_struct_command(peripheral, characteristic, 0x21, &fields).await;
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
