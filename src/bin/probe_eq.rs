#[path = "../logger.rs"]
pub mod logger;
#[path = "../service/ble.rs"]
pub mod ble;
#[path = "../service/midi.rs"]
pub mod midi;

use std::sync::mpsc;
use std::time::Duration;

// stub for ble.rs's `crate::model::flow8::MeterMsg` — only the variants the
// meter loop sends (the real enum pulls in the whole model tree)
pub mod model {
    pub mod flow8 {
        #[derive(Debug)]
        pub enum MeterMsg {
            Frame(Vec<f32>, usize, Vec<u8>),
            Fader { input: u8, output: u8, value: u8 },
            Flag { op: u8, id: u8, state: u8 },
        }
    }
}

/// Verify the ChannelMaster (0x06) dst ids for the remaining channel sends of
/// ch1: Mon1 (0x0A?), Mon2 (0x0B?), FX2 (0x0D?). FX1 = 0x0C is already
/// verified. Dump diffs of calibration-data/probe-dump-*.hex identify which
/// dump bytes each dst write lands on; final writes restore the −144 (off)
/// baseline.
fn main() {
    let (st, _sr) = mpsc::channel();
    let (sn, _nr) = mpsc::channel();
    let (mt, _mr) = mpsc::channel();
    let (se, _er) = mpsc::channel();

    let (sysex_tx, sysex_rx) = midi::create_sysex_channel();
    let inputs = midi::list_midi_input_devices();
    let input_dev = inputs.iter().find(|d| d.is_flow8).expect("FLOW 8 USB input");
    let _in_conn = midi::connect_input_device(input_dev.index, sysex_tx).expect("USB input connect");
    let outputs = midi::list_midi_output_devices();
    let out_dev = outputs.iter().find(|d| d.is_flow8).expect("FLOW 8 USB output");
    let _out = midi::connect_to_device(out_dev.index).expect("USB output connect");
    std::thread::spawn(move || {
        let mut n = 0;
        for data in sysex_rx {
            if data.len() < 200 {
                continue;
            }
            n += 1;
            let path = format!("calibration-data/probe-dump-{}.hex", n);
            let mut out = String::new();
            for (i, chunk) in data.chunks(16).enumerate() {
                out.push_str(&format!("{:04X}: ", i * 16));
                for b in chunk {
                    out.push_str(&format!("{:02X} ", b));
                }
                out.push('\n');
            }
            std::fs::write(&path, out).expect("write dump");
            println!("dump saved: {} ({} bytes)", path, data.len());
        }
    });

    println!("connecting...");
    let conn = ble::connect_flow8_ble(st, sn, mt, se).expect("BLE connect");
    println!("connected");

    let dump = |conn: &ble::BleConnection| {
        ble::send_dump_trigger(conn).expect("dump trigger");
        std::thread::sleep(Duration::from_secs(8));
    };

    dump(&conn); // dump1: baseline

    ble::send_channel_master(&conn, 0, 0x0A, 10.0).expect("W1");
    println!("W1 sent: dst 0x0A +10 dB");
    std::thread::sleep(Duration::from_secs(3));
    dump(&conn); // dump2

    ble::send_channel_master(&conn, 0, 0x0B, 10.0).expect("W2");
    println!("W2 sent: dst 0x0B +10 dB");
    std::thread::sleep(Duration::from_secs(3));
    dump(&conn); // dump3

    ble::send_channel_master(&conn, 0, 0x0D, 10.0).expect("W3");
    println!("W3 sent: dst 0x0D +10 dB");
    std::thread::sleep(Duration::from_secs(3));
    dump(&conn); // dump4

    // restore: all three baseline sends are −144 (off)
    ble::send_channel_master(&conn, 0, 0x0A, -144.0).expect("R1");
    std::thread::sleep(Duration::from_secs(2));
    ble::send_channel_master(&conn, 0, 0x0B, -144.0).expect("R2");
    std::thread::sleep(Duration::from_secs(2));
    ble::send_channel_master(&conn, 0, 0x0D, -144.0).expect("R3");
    std::thread::sleep(Duration::from_secs(2));
    dump(&conn); // dump5

    println!("done");
}
