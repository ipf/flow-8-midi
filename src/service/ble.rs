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
    settings_tx: Option<mpsc::Sender<(u8, u8)>>,
    mut notification_stream: NotifyStream,
) -> Result<(Vec<Option<String>>, NotifyStream), String> {
    use tokio_stream::StreamExt;

    log!("[BLE] Draining state dump (0x38)...");
    let mut dump_chunks = 0u32;
    let drain_deadline = std::time::Instant::now() + Duration::from_secs(3);
    while dump_chunks < 4 {
        let wait = drain_deadline.saturating_duration_since(std::time::Instant::now());
        if wait.is_zero() {
            break;
        }
        match tokio::time::timeout(wait, notification_stream.next()).await {
            Ok(Some(notification)) => {
                if !notification.value.is_empty() {
                    let ptype = notification.value[0];
                    if ptype == STATE_DUMP_TYPE {
                        // full hex of the mixer-state chunks: the channel
                        // on/mute states (leftOn|rightOn flags per 12 slots,
                        // FX1 = slot 10) live here, unparsed so far
                        let hex: Vec<String> =
                            notification.value.iter().map(|b| format!("{:02X}", b)).collect();
                        log!("[BLE] MixerState chunk (len={}): {}", notification.value.len(), hex.join(" "));
                        dump_chunks += 1;
                    } else {
                        log_debug!(
                            "[BLE] Notification: type=0x{:02X}, len={}",
                            ptype,
                            notification.value.len()
                        );
                    }
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    if dump_chunks >= 4 {
        log!("[BLE] State dump received ({} chunks)", dump_chunks);
    } else {
        log_warn!(
            "[BLE] State dump incomplete ({} chunks), proceeding anyway",
            dump_chunks
        );
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
        return Ok((Vec::new(), notification_stream));
    }

    let mut names: Option<Vec<Option<String>>> = None;
    let name_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while names.is_none() {
        let wait = name_deadline.saturating_duration_since(std::time::Instant::now());
        if wait.is_zero() {
            break;
        }
        match tokio::time::timeout(wait, notification_stream.next()).await {
            Ok(Some(notification)) => {
                if !notification.value.is_empty() {
                    let ptype = notification.value[0];
                    log_debug!(
                        "[BLE] Notification: type=0x{:02X}, len={}",
                        ptype,
                        notification.value.len()
                    );
                    if ptype == SNAPSHOT_RESPONSE_TYPE {
                        names = Some(parse_snapshot_names(&notification.value));
                    }
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    match names {
        Some(names) => {
            let named_count = names.iter().filter(|n| n.is_some()).count();
            log!(
                "[BLE] Parsed {} snapshot slots ({} with names)",
                names.len(),
                named_count
            );
            Ok((names, notification_stream))
        }
        None => {
            log_warn!("[BLE] Timeout waiting for snapshot names (5s)");
            Ok((Vec::new(), notification_stream))
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

    let result = rt.block_on(async {
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

        let (names, notification_stream) = match fetch_snapshot_names(
            &peripheral,
            &characteristic,
            Some(settings_tx),
            notification_stream,
        )
        .await
        {
            Ok(x) => x,
            Err(e) => {
                log_warn!("[BLE] Failed to get snapshot names: {}", e);
                return Ok(BleConnection {
                    peripheral,
                    characteristic,
                });
            }
        };

        if !names.is_empty() {
            let _ = snapshot_tx.send(names);
        } else {
            log_warn!("[BLE] No snapshot names received");
        }

        // the meter loop rides the authenticated main session's notification
        // stream (a second subscribe() on the same connection stalls on BlueZ)
        // — spawned on THIS runtime: a separate thread + runtime yields zero
        // notifications (the stream's forwarding dies with the runtime that
        // created it)
        tokio::spawn(meter_loop(
            peripheral.clone(),
            characteristic.clone(),
            notification_stream,
            meter_tx,
        ));

        Ok(BleConnection {
            peripheral,
            characteristic,
        })
    });

    // the meter task above lives on this runtime — dropping it here would
    // cancel the loop the moment connect returns
    std::mem::forget(rt);

    result
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

/// Compact commands (0x05 rename, 0x12/0x20 snapshot, 0x19/0x32 save/rename,
/// 0x25 setting, 0x40 tempo): same official framing as the struct commands —
/// every app command goes through xairbt_out_queue_write_cmd.
pub fn send_ble_command(conn: &BleConnection, opcode: u8, payload: &[u8]) -> Result<(), String> {
    send_ble_struct_command(conn, opcode, payload)
}

/// Struct commands (0x11 FX setup, 0x17 input preset, 0x21 meter request,
/// 0x31 FX preset, 0x41 select output): go out in the official 6-byte-header
/// chunked framing (see write_struct_command).
pub fn send_ble_struct_command(conn: &BleConnection, opcode: u8, payload: &[u8]) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("Tokio runtime error: {}", e))?;
    rt.block_on(async {
        write_struct_command(&conn.peripheral, &conn.characteristic, opcode, payload).await;
        Ok(())
    })
}

async fn write_struct_command(peripheral: &Peripheral, characteristic: &Characteristic, opcode: u8, payload: &[u8]) {
    static SEQ: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

    // official TX framing (xairbt_out_queue_write_cmd @0xec4e8 — every app
    // command goes out this way): [chunk_len u16][type][chunk_idx]
    // [chunk_count][seq] + data, chunk_size = type − 3, seq 0 for a
    // single-chunk command. The chunk data is the payload DIRECTLY (the
    // serializer copies the cmd's data area at +24): the zero-padded bare
    // form buried it and the [type][0x01][data][cksum] single form — accepted
    // only for the session commands 0x37/0x39/0x07/0x4B — never got a
    // 0x21/0x41 response in any tested shape.
    let chunk_size = (opcode as usize).saturating_sub(3).max(1);
    let count = payload.len().div_ceil(chunk_size);
    let seq = if count > 1 {
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    } else {
        0
    };
    for (idx, chunk) in payload.chunks(chunk_size).enumerate() {
        let mut p = Vec::with_capacity(chunk.len() + 6);
        p.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
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

/// UI fader CC (0..=127) → dB on the −70…+10 scale (0 = OFF → −144).
/// Inverse of fix8_to_level_cc; used to rebuild fix8 wire values from the
/// model's stored CC levels.
pub fn level_cc_to_db(cc: u8) -> f32 {
    if cc == 0 {
        -144.0
    } else {
        -70.0 + (cc as f32 - 1.0) / 126.0 * 80.0
    }
}

/// FX channel state (0x30): the only documented writer that carries the FX
/// return level (table 36 `vol` @0x8) — ChannelMaster 0x0C/0x0D→self and USB
/// CC#7 on raw 10/11 are both dump-proven silently ignored for FX returns.
/// Compact wire form (the serializer emits only the table's fields; type-6
/// bools bit-pack 8/byte, the type-11 gains walk nested table 24 = one fix8
/// sel-3 byte each, no count byte):
/// [id][flags][vol fix8 sel3][pan fix8 sel1][val1][val2][val3][preset][7×gain]
/// `preset_raw` is the dump's 1..=16 representation.
#[allow(clippy::too_many_arguments)]
pub fn send_fx_channel_state(
    conn: &BleConnection,
    fx_ch_id: u8,
    muted: bool,
    enable_main: bool,
    enable_mon1: bool,
    enable_mon2: bool,
    vol_db: f32,
    pan: u8,
    val1: u8,
    val2: u8,
    val3: u8,
    preset_raw: u8,
    gains_db: &[f32; 7],
) -> Result<(), String> {
    let flags = muted as u8
        | ((enable_main as u8) << 1)
        | ((enable_mon1 as u8) << 2)
        | ((enable_mon2 as u8) << 3);
    let mut payload = Vec::with_capacity(15);
    payload.push(fx_ch_id);
    payload.push(flags);
    payload.push(db_to_fix8(vol_db));
    payload.push(pan);
    payload.push(val1);
    payload.push(val2);
    payload.push(val3);
    payload.push(preset_raw);
    for db in gains_db {
        payload.push(db_to_fix8(*db));
    }
    send_ble_struct_command(conn, 0x30, &payload)
}

// ── VU meters: MeterRequest (0x21); device streams MeterUpdate (0x22) ──
const METER_UPDATE_TYPE: u8 = 0x22;
// Official MeterRequest payload (field table 28): count byte + flat 15-byte
// id table. setMeterOutputChannelId @0xb1564 copies 10 bytes from
// METER_REQUEST_CHANNELS @0x10cc70 — [40 41 42 43 C4 C5 C6 <out> 00 00] —
// with the selected output (id|0x40|0x80 stereo) overwriting index 7; the
// remaining 5 table slots stay zero. The device reports the selected output's
// L/R in update floats 10/11. Wire framing: [type][0x01][payload][byte-sum]
// — proven on-air by the 0x39 auth (cksum 0x2A) and [37][01][38].
const METER_INPUT_IDS: [u8; 7] = [0x40, 0x41, 0x42, 0x43, 0xC4, 0xC5, 0xC6];
/// metered outputs: output channel id | 0x40 | 0x80 (stereo) —
/// Main 0x0F→0xCF, Mon 1 0x18→0xD8, Mon 2 0x19→0xD9 (raw bus ids 7/8/9 are
/// the wrong id space)
const METER_OUTPUT_BYTES: [u8; 3] = [0xCF, 0xD8, 0xD9];

// Fader/meter dB curve: fix8 selector 3 lookup (F48 from the official
// app's tables — a meter level byte OR a fader position byte b means
// F48[b] dB; F48's zero is at index ~191, so 0 dB = fix8 190/191 —
// unrelated to the sysex CC scale where 0 dB = CC 111).
pub const F48: [f32; 256] = [
    -144.0, -88.11764526367188, -86.23529052734375, -84.35294342041016, -82.47058868408203, -80.5882339477539, -78.70587921142578, -76.82353210449219,
    -74.94117736816406, -73.05882263183594, -71.17646789550781, -69.29411315917969, -67.4117660522461, -65.52941131591797, -63.647056579589844, -61.764705657958984,
    -59.960784912109375, -59.33333206176758, -58.70588302612305, -58.07843017578125, -57.45098114013672, -56.82352828979492, -56.19607925415039, -55.568626403808594,
    -54.94117736816406, -54.313724517822266, -53.68627166748047, -53.05882263183594, -52.431373596191406, -51.80392074584961, -51.17646789550781, -50.54901885986328,
    -49.92156982421875, -49.29411697387695, -48.666664123535156, -48.039215087890625, -47.411766052246094, -46.7843132019043, -46.1568603515625, -45.52941131591797,
    -44.90196228027344, -44.27450942993164, -43.647056579589844, -43.01960754394531, -42.39215850830078, -41.764705657958984, -41.13725280761719, -40.509803771972656,
    -39.882354736328125, -39.25490188598633, -38.62744903564453, -38.0, -37.3725471496582, -36.74509811401367, -36.117645263671875, -35.490196228027344,
    -34.86274337768555, -34.235294342041016, -33.60784149169922, -32.98039245605469, -32.35293960571289, -31.72549057006836, -31.098037719726562, -30.47058868408203,
    -29.921567916870117, -29.60784149169922, -29.294116973876953, -28.980390548706055, -28.66666603088379, -28.35293960571289, -28.039215087890625, -27.725488662719727,
    -27.41176414489746, -27.098037719726562, -26.784313201904297, -26.4705867767334, -26.156862258911133, -25.843135833740234, -25.52941131591797, -25.21568489074707,
    -24.901960372924805, -24.588233947753906, -24.27450942993164, -23.960783004760742, -23.647058486938477, -23.333332061767578, -23.019607543945312, -22.705881118774414,
    -22.39215660095215, -22.07843017578125, -21.764705657958984, -21.450979232788086, -21.13725471496582, -20.823528289794922, -20.509803771972656, -20.196077346801758,
    -19.882352828979492, -19.568626403808594, -19.254901885986328, -18.94117546081543, -18.627450942993164, -18.313724517822266, -18.0, -17.686275482177734,
    -17.372547149658203, -17.058822631835938, -16.745098114013672, -16.431373596191406, -16.117645263671875, -15.80392074584961, -15.490196228027344, -15.176471710205078,
    -14.862743377685547, -14.549018859863281, -14.235294342041016, -13.92156982421875, -13.607840538024902, -13.294116973876953, -12.980392456054688, -12.666667938232422,
    -12.35293960571289, -12.039215087890625, -11.72549057006836, -11.411766052246094, -11.098037719726562, -10.784313201904297, -10.470588684082031, -10.156864166259766,
    -9.921567916870117, -9.764705657958984, -9.607840538024902, -9.450979232788086, -9.294116973876953, -9.137252807617188, -8.980390548706055, -8.823528289794922,
    -8.666666030883789, -8.509803771972656, -8.35293960571289, -8.196077346801758, -8.039215087890625, -7.882350921630859, -7.725489139556885, -7.5686259269714355,
    -7.411764144897461, -7.254901885986328, -7.098038196563721, -6.9411749839782715, -6.784313201904297, -6.627449035644531, -6.470586776733398, -6.313724994659424,
    -6.156861782073975, -6.0, -5.843135833740234, -5.68627405166626, -5.5294108390808105, -5.372547149658203, -5.21568489074707, -5.058823108673096,
    -4.9019598960876465, -4.745098114013672, -4.588233947753906, -4.431372165679932, -4.274508953094482, -4.117644786834717, -3.960783004760742, -3.8039209842681885,
    -3.6470580101013184, -3.4901959896087646, -3.333332061767578, -3.1764700412750244, -3.0196080207824707, -2.8627429008483887, -2.705881118774414, -2.5490190982818604,
    -2.3921570777893066, -2.2352941036224365, -2.078429937362671, -1.9215680360794067, -1.764706015586853, -1.6078410148620605, -1.4509789943695068, -1.2941169738769531,
    -1.1372549533843994, -0.9803919792175293, -0.823527991771698, -0.6666659712791443, -0.5098040103912354, -0.35293999314308167, -0.19607700407505035, 0.0,
    0.11764699965715408, 0.2745090126991272, 0.4313740134239197, 0.5882359743118286, 0.7450979948043823, 0.9019619822502136, 1.0588250160217285, 1.2156870365142822,
    1.372549057006836, 1.5294109582901, 1.6862750053405762, 1.8431379795074463, 2.0, 2.1568639278411865, 2.313724994659424, 2.4705889225006104,
    2.627453088760376, 2.7843129634857178, 2.9411768913269043, 3.0980379581451416, 3.254901885986328, 3.4117660522460938, 3.5686259269714355, 3.7254910469055176,
    3.882354974746704, 4.039215087890625, 4.196078777313232, 4.352942943572998, 4.509803771972656, 4.666667938232422, 4.823527812957764, 4.980391979217529,
    5.137257099151611, 5.294116973876953, 5.450981140136719, 5.6078410148620605, 5.764706134796143, 5.92156982421875, 6.07843017578125, 6.235293865203857,
    6.3921589851379395, 6.549018859863281, 6.705883026123047, 6.8627471923828125, 7.019608020782471, 7.176472187042236, 7.333332061767578, 7.490196228027344,
    7.647059917449951, 7.803921222686768, 7.960784912109375, 8.117645263671875, 8.27450942993164, 8.431373596191406, 8.588233947753906, 8.745098114013672,
    8.901962280273438, 9.058822631835938, 9.215686798095703, 9.372550964355469, 9.529411315917969, 9.686274528503418, 9.843135833740234, 10.0,
];

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

/// Fader/sends fix8 byte (F48 dB index, selector 3) → the UI's 1..=127 CC
/// scale (−70…+10 dB, 0 dB = CC 111). The 0x06 fader push reports fix8, not
/// CC — writing it raw made a 0 dB fader read +50 dB (fix8 190).
/// Corroborated on air: CC 111 ↔ echo 190, CC 92 ↔ 120, CC 1 ↔ 10,
/// CC 127 ↔ 250.
pub fn fix8_to_level_cc(b: u8) -> u8 {
    let db = F48[b as usize];
    if db < -70.0 {
        return 0; // OFF
    }
    let cc = (1.0 + (db + 70.0) / 80.0 * 126.0).round() as i32;
    cc.clamp(1, 127) as u8
}

/// dB value → fix8 byte (F48 index, selector 3 = first table step above the
/// value; native xairbt_float_to_fix8, confirmed against XairbtProtocol.kt).
pub fn db_to_fix8(db: f32) -> u8 {
    for (i, &step) in F48.iter().enumerate().skip(1) {
        if db < step {
            return (i - 1) as u8;
        }
    }
    255
}

/// ChannelMaster (0x06) TX: [id][dst][fix8(dB)] — the official app's path for
/// every fader move (queueChannelMasterControlCommandPackets @0x79cd0: id at
/// cmd+0x18, dst at +0x19, dB f32 at +0x1C; wire layout = table 7 via
/// CMD_FT[0x06], the f32 serialized as fix8 selector 3). dst == id sets the
/// bus's own fader (FX return level); dst = 0x0C/0x0D sets id's FX send.
pub fn send_channel_master(conn: &BleConnection, id: u8, dst: u8, db: f32) -> Result<(), String> {
    send_ble_struct_command(conn, 0x06, &[id, dst, db_to_fix8(db)])
}

/// ChannelMute (0x08) TX: [id][flag] — works on input channels AND on the FX
/// buses: the official app's FX mute button sends ChannelMute on
/// FXBusChannel1/2Id (0x0C/0x0D) (queueUpdateFXChannelMutesCommandPackets).
pub fn send_channel_mute(conn: &BleConnection, id: u8, muted: bool) -> Result<(), String> {
    send_ble_struct_command(conn, 0x08, &[id, muted as u8])
}

/// Run the meter receive loop on its own thread, riding the main session's
/// notification stream (a second subscribe() on the same connection stalls on
/// BlueZ; the mixer answers meter polls only on an authenticated connection).
/// Meter loop: consumes the main session's notification stream and polls the
/// MeterRequest every iteration. Runs as a tokio task on the connect runtime —
/// a separate thread + runtime never received a single notification (the
/// stream's forwarding dies with the runtime that created it).
async fn meter_loop(
    peripheral: Peripheral,
    characteristic: Characteristic,
    mut notifications: NotifyStream,
    meter_tx: mpsc::Sender<MeterMsg>,
) {
    use tokio_stream::StreamExt;

    let mut first_data = true;
    let mut other_logged = 0u32;
    let mut samples_logged = 0u32;
    let mut frames_logged = 0u32;
    let mut frames_seen = 0u32;
    log!("[BLE] Meter loop running (main session stream)");
    // rotating output select: poll 1 meters Main (0xCF), then Mon1 (0xD8)
    // and Mon2 (0xD9) — slots 10/11 always carry the SELECTED output's L/R
    // (disasm 0xb1564: the output id overwrites payload index 7), and
    // out_rot 0/1/2 maps to buses Main/Mon1/Mon2 in the UI consumer
    // FLOW8_METER_OUT (hex) pins the poll to one output id — e.g. CC meters
    // the FX1 bus directly (0x0C|0x40|0x80) to test the FX chain internally
    let pinned_out = std::env::var("FLOW8_METER_OUT").ok().and_then(|s| {
        u8::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
    });
    let mut out_rot = 0usize;
    send_meter_poll(&peripheral, &characteristic, pinned_out.unwrap_or(METER_OUTPUT_BYTES[out_rot])).await;
    'outer: loop {
        // one poll in flight: the device paces exactly one 0x22 frame per
        // accepted poll (~410 ms on its own timer — frames arrive 410 ms
        // apart whether polled at 2.5 Hz, ~5 Hz, or not polled at all), so
        // out_rot advances ONLY when a frame lands: poll↔frame stay 1:1
        // and each frame's rotation index is the poll's output
        let deadline = tokio::time::Instant::now() + Duration::from_millis(700);
        let mut got_frame = false;
        while !got_frame {
            let wait = deadline.saturating_duration_since(tokio::time::Instant::now());
            if wait.is_zero() {
                // frame missed (poll lost or stream hiccup): re-poll the
                // SAME output — don't advance, assignment stays exact
                break;
            }
            let notification = match tokio::time::timeout(wait, notifications.next()).await {
                Ok(Some(n)) => Some(n),
                Ok(None) => {
                    log_warn!("[BLE] Meter loop: notification stream ended");
                    break 'outer;
                }
                Err(_) => None,
            };
            let Some(notification) = notification else { break };
            let v = &notification.value;
            if v.first() == Some(&METER_UPDATE_TYPE) {
                // raw hex of the first frames: the payload shape is
                // still unknown (len=20, not the 63-byte table-30 form)
                if frames_logged < 5 {
                    frames_logged += 1;
                    let hex: Vec<String> =
                        v.iter().map(|b| format!("{:02X}", b)).collect();
                    log!(
                        "[BLE] Meter frame #{} (len={}): {}",
                        frames_logged,
                        v.len(),
                        hex.join(" ")
                    );
                }
                // [22][01][15 × fix8 dB bytes (F48 lookup)][GR u16 BE]
                // [byte-sum cksum] — fix8 = 1 BYTE on the wire
                // (XairbtProtocol selector 3 = F48 index), confirmed
                // by the 20-byte air frame and XairbtQueues.kt
                let mut values = Vec::with_capacity(15);
                let mut gr: u16 = 0;
                if v.len() >= 20 {
                    for &b in &v[2..17] {
                        values.push(F48[b as usize]);
                    }
                    gr = u16::from_be_bytes([v[17], v[18]]);
                }
                if first_data {
                    first_data = false;
                    let floats: Vec<String> =
                        values.iter().map(|f| format!("{:.1}", f)).collect();
                    log!(
                        "[BLE] Meter data flowing ({} slots, GR={}): {}",
                        values.len(),
                        gr,
                        floats.join(" ")
                    );
                }
                frames_seen += 1;
                if frames_seen % 4 == 0 {
                    let dbv: Vec<String> =
                        values.iter().map(|f| format!("{:.0}", f)).collect();
                    log_debug!(
                        "[BLE] Meter @{}: {}",
                        frames_seen,
                        dbv.join(" ")
                    );
                }
                // GR u16 unpack per native XAIRClient::updateMeters: inputs
                // get bits 2*chIdx, the metered output the top 2 bits
                // (2-bit values 0..=3); slots 12-14 don't map to channels
                // (meterIndexToChannelIndex → -1 for i ≥ 12)
                let mut gr_slots = Vec::with_capacity(8);
                for i in 0..7u32 {
                    gr_slots.push(((gr >> (2 * i)) & 3) as u8);
                }
                gr_slots.push(((gr >> 14) & 3) as u8);
                let _ = meter_tx.send(MeterMsg::Frame(values, pinned_out.map(|_| 0).unwrap_or(out_rot), gr_slots));
                out_rot = (out_rot + 1) % METER_OUTPUT_BYTES.len();
                got_frame = true;
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
        // the device freed the poll slot (frame landed or missed): send
        // the next request immediately — the device paces the answer itself
        send_meter_poll(&peripheral, &characteristic, pinned_out.unwrap_or(METER_OUTPUT_BYTES[out_rot])).await;
    }
}

/// MeterRequest poll (0x21) — the device's metering trigger is the RAW
/// SINGLE FORM [type][0x01][payload][byte-sum], NOT the queued-chunk framing:
/// on-air proof — the 13:54 session polled the single form for 14 min and the
/// device was still streaming meters at the NEXT connect (89 × 0x22 frames in
/// its handshake phase); 9 min of queued-form polls produced zero. Payload =
/// field table 28: count byte + flat 15-byte id table (7 inputs + selected
/// output + 7 zeros). The retry's disconnect stops metering, so the poll
/// re-sends every iteration.
async fn send_meter_poll(peripheral: &Peripheral, characteristic: &Characteristic, output: u8) {
    let mut packet = vec![0x21, 0x01, 0x08];
    packet.extend_from_slice(&METER_INPUT_IDS);
    packet.push(output); // selected output: id|0x40|0x80 (see poll rotation)
    packet.extend_from_slice(&[0x00; 7]);
    let cksum = packet.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    packet.push(cksum);
    write_packet(peripheral, characteristic, &packet, 0x21).await;
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
        match fetch_snapshot_names(&conn.peripheral, &conn.characteristic, None, stream).await {
            Ok((names, _stream)) => names,
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
