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

/// Live check for the ChannelMaster TX path: connects, sets the FX1 return
/// (ChannelMaster 0x0C→0x0C) to +10 dB, then waits for the device's fader
/// push echo ([06][01][0C][0C][fix8][cksum], logged by the meter loop).
fn main() {
    let (st, _sr) = mpsc::channel();
    let (sn, _nr) = mpsc::channel();
    let (mt, mr) = mpsc::channel();
    let (se, _er) = mpsc::channel();

    println!("connecting...");
    let conn = ble::connect_flow8_ble(st, sn, mt, se).expect("BLE connect");
    println!("connected; sending ChannelMaster(0x0C, 0x0C, +10 dB)");
    ble::send_channel_master(&conn, 0x0C, 0x0C, 10.0).expect("CM send");
    println!("sent; waiting up to 6 s for the fader push echo...");
    std::thread::sleep(Duration::from_secs(6));
    while let Ok(m) = mr.try_recv() {
        println!("{m:?}");
    }
    println!("done");
}
