//! Diagnostic: subscribe to ALL notify characteristics of the connected
//! FLOW 8 and hex-dump every notification for N seconds (arg 1, default 30).
//! Rides on the existing BlueZ connection — the running app is untouched.
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager};
use std::time::Duration;

const TARGET: &str = "FLOW 8 LE";

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(run(secs));
}

async fn run(secs: u64) {
    let manager = Manager::new().await.unwrap();
    let adapters = manager.adapters().await.unwrap();
    let adapter: Adapter = adapters.into_iter().next().unwrap();
    adapter.start_scan(ScanFilter::default()).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    let mut peripheral = None;
    for p in adapter.peripherals().await.unwrap() {
        if let Some(prop) = p.properties().await.unwrap_or_default() {
            if prop.local_name.as_deref() == Some(TARGET) {
                peripheral = Some(p);
                break;
            }
        }
    }
    let peripheral = peripheral.expect("FLOW 8 LE not found");
    if !peripheral.is_connected().await.unwrap() {
        peripheral.connect().await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    peripheral.discover_services().await.unwrap();
    let chars = peripheral.characteristics();
    let mut notified = 0;
    for c in &chars {
        let props = format!("{:?}", c.properties);
        if props.contains("NOTIFY") || props.contains("INDICATE") {
            println!("SUBSCRIBE uuid={} {}", c.uuid, props);
            if peripheral.subscribe(c).await.is_ok() {
                notified += 1;
            }
        }
    }
    println!("subscribed {} characteristics", notified);
    let mut stream = peripheral.notifications().await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    use tokio_stream::StreamExt;
    while let Ok(Some(n)) = tokio::time::timeout_at(deadline, stream.next()).await {
        let hex: Vec<String> = n.value.iter().map(|b| format!("{:02x}", b)).collect();
        println!(
            "{} uuid={} len={} {}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
                % 100000,
            n.uuid,
            n.value.len(),
            hex.join(" ")
        );
    }
    println!("probe done");
}
