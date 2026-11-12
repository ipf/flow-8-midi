#[path = "../logger.rs"]
pub mod logger;
#[path = "../service/ble.rs"]
pub mod ble;

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
        }
    }
}

/// Live check of the whole FX chain: loads the AMBIENCE reverb on FX1
/// (FXLoadPreset 0x31, channelId 0x0C, 0-based preset 0), then toggles the
/// FX1 return (ChannelMaster 0x0C→0x0C) between +10 dB and −144 while a tone
/// plays through USB. The Main output meter (MeterUpdate slots 10/11, logged
/// by the meter loop) must change when the return toggles — that proves the
/// send → engine → return → Main chain end-to-end, no ears needed.
fn main() {
    let (st, _sr) = mpsc::channel();
    let (sn, _nr) = mpsc::channel();
    let (mt, mr) = mpsc::channel();
    let (se, _er) = mpsc::channel();

    println!("connecting...");
    let conn = ble::connect_flow8_ble(st, sn, mt, se).expect("BLE connect");
    println!("connected; loading FX presets (FX1 reverb 0, FX2 delay 0)");
    ble::send_fx_preset(&conn, 0x0C, 0).expect("fx1 preset");
    ble::send_fx_preset(&conn, 0x0D, 0).expect("fx2 preset");
    std::thread::sleep(Duration::from_secs(2));

    println!("parking FX returns at OFF (-144 dB)");
    ble::send_channel_master(&conn, 0x0C, 0x0C, -144.0).expect("fx1 return");
    ble::send_channel_master(&conn, 0x0D, 0x0D, -144.0).expect("fx2 return");
    std::thread::sleep(Duration::from_secs(1));

    while let Ok(m) = mr.try_recv() {
        println!("{m:?}");
    }
    println!("done");
}
