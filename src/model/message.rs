use super::channels::{BusId, BusIdx, ChannelId, FxSlotId};
use super::flow8::SyncInterval;
use super::page::Page;
use iced::Theme;

#[derive(Debug, Clone)]
pub enum InterfaceMessage {
    NavigateTo(Page),
    RetryConnection,
    Disconnect,

    Mute(ChannelId, bool),
    Solo(ChannelId, bool),
    Gain(ChannelId, u8),
    Level(ChannelId, u8),
    /// Apply an input preset (index into model::presets::INPUT_PRESETS)
    LoadInputPreset(ChannelId, u8),
    Balance(ChannelId, u8),
    LowCut(ChannelId, u8),
    Compressor(ChannelId, u8),
    PhantomPower(ChannelId),

    EqLow(ChannelId, u8),
    EqLowMid(ChannelId, u8),
    EqHiMid(ChannelId, u8),
    EqHi(ChannelId, u8),

    SendMon1(ChannelId, u8),
    SendMon2(ChannelId, u8),
    SendFx1(ChannelId, u8),
    SendFx2(ChannelId, u8),

    BusLevel(BusIdx, BusId, u8),
    BusBalance(BusIdx, BusId, u8),
    BusLimiter(BusIdx, BusId, u8),
    BusNineBandEq(BusIdx, BusId, u8, u8),
    MonitorTab(usize),

    FxPreset(FxSlotId, u8),
    FxParam1(FxSlotId, u8),
    FxParam2(FxSlotId, u8),
    /// slider release: send the settled FX params (0x11) once
    FxParam1Send(FxSlotId),
    FxRouting(FxSlotId, bool, bool, bool),
    FxMute,
    TapTempo,

    LoadSnapshot(u8),
    ResetMixer,

    RenameStartChannel(ChannelId),
    RenameStartSnapshot(usize),
    SnapshotSaveStart(usize),
    RenameChanged(String),
    RenameSubmit,
    RenameCancel,
    DeleteSnapshot(usize),
    /// Device setting write: id byte (0x01..=0x0f) and value byte
    SetDeviceSetting(u8, u8),

    ThemeChanged(Theme),
    SyncIntervalChanged(SyncInterval),
    OpenManual,
    OpenRepository,
    OpenDonation,
    CopyDebugLog,
    SaveDebugLog,
    #[cfg(any(debug_assertions, feature = "dev-tools"))]
    CopyHexDump,
    Tick,
    BleConnect,
    BleRequestDump,
    #[cfg(any(debug_assertions, feature = "dev-tools"))]
    CalibrateStart,
    #[cfg(any(debug_assertions, feature = "dev-tools"))]
    DigestRun,
}
