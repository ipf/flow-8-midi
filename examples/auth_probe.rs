//! Standalone auth + meter probe. Connects to the FLOW 8, runs the app's
//! handshake (auth key, session start, 0x37 challenge wait — 10 s, not 3),
//! then polls meters (0x41 select + 0x21 every 250 ms) and hex-dumps every
//! notification for 30 s. Answers three questions without the app:
//! challenge timing (pairing-window need), poll behavior, record format.
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Adapter, Manager, Peripheral};
use std::time::Duration;

// from src/service/ble.rs — the paired client key
const AUTH_PACKET: [u8; 19] = [
    0x39, 0x01, 0xFD, 0x06, 0x2B, 0x06, 0x39, 0xF1, 0x7F, 0xE7, 0xB7, 0x27, 0x8B, 0x8F, 0x35,
    0x5A, 0x49, 0x5C, 0x2A,
];
const SESSION_START_PACKET: [u8; 3] = [0x37, 0x01, 0x38];
const METER_INPUT_IDS: [u8; 7] = [0x40, 0x41, 0x42, 0x43, 0xC4, 0xC5, 0xC6];

fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(run());
}

async fn run() {
    let manager = Manager::new().await.unwrap();
    let adapter = manager.adapters().await.unwrap().into_iter().next().unwrap();
    adapter.start_scan(ScanFilter::default()).await.unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    let mut target: Option<Peripheral> = None;
    for p in adapter.peripherals().await.unwrap() {
        if let Some(prop) = p.properties().await.unwrap_or_default() {
            if prop.local_name.as_deref() == Some("FLOW 8 LE") {
                target = Some(p);
                break;
            }
        }
    }
    let peripheral = target.expect("FLOW 8 LE not found");
    peripheral.connect().await.expect("connect failed");
    tokio::time::sleep(Duration::from_secs(1)).await;
    peripheral.discover_services().await.unwrap();
    let characteristic = peripheral
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == uuid::Uuid::parse_str("0034594a-a8e7-4b1a-a6b1-cd5243059a57").unwrap())
        .expect("characteristic not found");
    peripheral.subscribe(&characteristic).await.unwrap();
    println!("connected + subscribed, t=0");

    let t0 = std::time::Instant::now();
    let stamp = || format!("[{:>6}.{}] t=+{:.1}s", t0.elapsed().as_millis(), 0, t0.elapsed().as_secs_f32());

    // 1. auth key
    peripheral.write(&characteristic, &AUTH_PACKET, WriteType::WithResponse).await.unwrap();
    println!("{} auth key sent", stamp());
    // 2. session start
    tokio::time::sleep(Duration::from_millis(300)).await;
    peripheral.write(&characteristic, &SESSION_START_PACKET, WriteType::WithResponse).await.unwrap();
    println!("{} session start sent — waiting up to 10 s for 0x37 challenge", stamp());

    // 3. challenge wait, 10 s, logging EVERY notification as it comes
    let mut stream = peripheral.notifications().await.unwrap();
    use tokio_stream::StreamExt;
    let mut challenged = false;
    {
        let wait = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(n) = stream.next().await {
                println!("{} notify: {}", stamp(), hex(&n.value));
                if n.value.first() == Some(&0x37) {
                    challenged = true;
                    break;
                }
            }
        })
        .await;
        if wait.is_err() {
            println!("{} no challenge in 10 s", stamp());
        }
    }
    if challenged {
        let reply: Vec<u8> = [0x39u8].iter().copied().chain(AUTH_PACKET[2..18].iter().copied()).collect();
        peripheral.write(&characteristic, &reply, WriteType::WithoutResponse).await.ok();
        println!("{} challenge received — replied with key", stamp());
    } else {
        println!("{} proceeding WITHOUT challenge (device did not ask)", stamp());
    }

    // 4. meter polls: select Main, then 0x21 every 250 ms for 25 s
    println!("{} starting meter polls (0x41 once, 0x21 every 250 ms)", stamp());
    let mut last_poll = std::time::Instant::now();
    let mut selected = false;
    let deadline = t0 + Duration::from_secs(35);
    let mut count = 0usize;
    let mut seen = 0usize;
    while std::time::Instant::now() < deadline {
        if !selected {
            write_struct(&peripheral, &characteristic, 0x41, &[7u8]).await;
            selected = true;
        }
        if last_poll.elapsed() >= Duration::from_millis(250) {
            let fields = [
                0x08u8, METER_INPUT_IDS[0], METER_INPUT_IDS[1], METER_INPUT_IDS[2],
                METER_INPUT_IDS[3], METER_INPUT_IDS[4], METER_INPUT_IDS[5],
                METER_INPUT_IDS[6], 0xC7u8, 0x00, 0x00,
            ];
            write_struct(&peripheral, &characteristic, 0x21, &fields).await;
            last_poll = std::time::Instant::now();
            count += 1;
        }
        // drain notifications without blocking
        match tokio::time::timeout(Duration::from_millis(10), stream.next()).await {
            Ok(Some(n)) => {
                seen += 1;
                if seen <= 60 {
                    println!("{} notify: {}", stamp(), hex(&n.value));
                }
            }
            _ => {}
        }
    }
    println!("{} done: {} polls sent, {} notifications seen", stamp(), count, seen);
}

/// Same wire form as the app's write_struct_command: bare-padded when it fits
/// 20 bytes, else [len u16][type][idx][count][seq] chunks of 17 data bytes.
async fn write_struct(peripheral: &Peripheral, characteristic: &btleplug::api::Characteristic, opcode: u8, payload: &[u8]) {
    static SEQ: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    if 24 + payload.len() <= 20 {
        let mut bare = vec![0u8; 24 + payload.len()];
        bare[0] = opcode;
        bare[24..].copy_from_slice(payload);
        peripheral.write(characteristic, &bare, WriteType::WithoutResponse).await.ok();
        return;
    }
    let mut fields = vec![0u8; 23 + payload.len()];
    fields[23..].copy_from_slice(payload);
    const CHUNK: usize = 17;
    let count = fields.len().div_ceil(CHUNK);
    let seq = if count > 1 {
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    } else {
        0
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
        peripheral.write(characteristic, &p, WriteType::WithoutResponse).await.ok();
    }
}
