#[path = "../logger.rs"]
pub mod logger;
#[path = "../service/midi.rs"]
pub mod midi;

/// USB-only dump listener: the running app triggers state dumps via BLE
/// (trigger works only via BLE) and the dump arrives on USB — this catches
/// the same dumps alongside the app to verify the device's real ch1 PEQ
/// state after knob moves.
fn main() {
    let (sysex_tx, sysex_rx) = midi::create_sysex_channel();
    let inputs = midi::list_midi_input_devices();
    let input_dev = inputs.iter().find(|d| d.is_flow8).expect("FLOW 8 USB input");
    let _in_conn = midi::connect_input_device(input_dev.index, sysex_tx).expect("USB input connect");
    println!("listening for USB SysEx dumps (app's periodic BLE sync triggers them)...");
    let mut n = 0;
    for data in sysex_rx {
        if data.len() < 2000 {
            continue;
        }
        n += 1;
        let path = format!("calibration-data/verify-dump-{}.hex", n);
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
        if n >= 2 {
            break;
        }
    }
    println!("done");
}
