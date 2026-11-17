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

/// Pin the Q=1.0 (byte2 0x80) parking spots for band 1 and band 4 — the two
/// bands whose Q byte2 parking is still unverified (band 1 Q=1.0 is the
/// official app's default, so it must decode right). Restores Q=0.0.
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
    let peq = |conn: &ble::BleConnection, band: u8, freq: u16, q_byte: u8, label: &str| {
        ble::send_ble_struct_command(
            conn,
            0x09,
            &[0, band, (freq >> 8) as u8, (freq & 0xFF) as u8, q_byte, 127],
        )
        .expect(label);
        println!("{} sent", label);
        std::thread::sleep(Duration::from_secs(3));
    };

    dump(&conn); // dump1: baseline

    peq(&conn, 0, 100, 8, "W1 band1 Q=1.0"); // dump2
    dump(&conn);
    peq(&conn, 3, 12000, 8, "W2 band4 Q=1.0"); // dump3
    dump(&conn);

    peq(&conn, 0, 100, 0, "R1 band1 Q=0.0");
    peq(&conn, 3, 12000, 0, "R2 band4 Q=0.0");
    dump(&conn); // dump4

    println!("done");
}
