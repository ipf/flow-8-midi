//! The 8 input-channel presets. Values extracted from the official Android
//! app's static table (loadInputChannelPresets @0xef440 in
//! libcom_musicgroup_xairbt.so): gain dB (ctor arg w7 → preset[+6]),
//! HPF Hz (ushort arg t → preset[+24]), phantom (bool arg w5 → preset[+4]).
//! All presets have pan center, compressor 0, no mute, no phase invert.

pub struct InputPreset {
    pub id: u8,
    pub name: &'static str,
    pub gain_db: i8,
    pub hpf_hz: u16,
    pub phantom: bool,
}

pub const INPUT_PRESETS: [InputPreset; 8] = [
    InputPreset { id: 1, name: "Acoustic Guitar", gain_db: 20, hpf_hz: 150, phantom: false },
    InputPreset { id: 2, name: "Electric Guitar", gain_db: 20, hpf_hz: 120, phantom: false },
    InputPreset { id: 3, name: "Bass Guitar", gain_db: 20, hpf_hz: 40, phantom: false },
    InputPreset { id: 4, name: "Vocal mic", gain_db: 45, hpf_hz: 120, phantom: false },
    InputPreset { id: 5, name: "Condenser Mic", gain_db: 35, hpf_hz: 120, phantom: true },
    InputPreset { id: 6, name: "Bass Drum", gain_db: 35, hpf_hz: 40, phantom: false },
    InputPreset { id: 7, name: "Keyboard", gain_db: 20, hpf_hz: 0, phantom: false },
    InputPreset { id: 8, name: "Snare Drum", gain_db: 35, hpf_hz: 180, phantom: false },
];
