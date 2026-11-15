# FLOW 8 MIDI Controller — User Manual

A non-official cross-platform desktop MIDI controller for the [Behringer FLOW 8](https://www.behringer.com/behringer/product?modelCode=0603-AEW) mixer.

***

## 1. What is FLOW 8 MIDI Controller?

FLOW 8 MIDI Controller is a desktop application that lets you control your Behringer FLOW 8 mixer from your computer via USB, in an SSL-style console layout.

**What it does:**

* Full control of all mixer parameters: gain, level, pan, EQ, sends, FX, mute, solo, compressor, low cut, phantom power, and limiter.
* FX control: preset selection, parameter editing, Main/Monitor routing, global FX mute, and tap tempo.
* Reads the current mixer state and reflects it in the UI (requires Bluetooth — see [Section 6](#6-bluetooth-ble-sync)).
* Loads mixer snapshots. Snapshot names are displayed when Bluetooth is connected.
* Tooltips display real parameter values (dB, Hz, %) matching the mixer's MIDI implementation.

**What it does NOT do:**

* It does not reflect physical knob/fader movements in real time. USB only allows sending commands to the mixer, not receiving its current state. Bluetooth partially solves this — see [Section 6](#6-bluetooth-ble-sync).
* It is not an official Behringer product.

***

## 2. Getting Started

### 2.1 Requirements

* A **Behringer FLOW 8** mixer connected via **USB** to your computer.
* **(Optional)** A Bluetooth adapter on your computer for BLE sync features (state sync, snapshot names).

> **Important:** The FLOW 8 only allows **one Bluetooth connection at a time**. If the official Behringer Android/iOS app is connected to the mixer, this application will not be able to establish a Bluetooth connection (and vice-versa). Make sure to disconnect from one before connecting with the other.

### 2.2 Connecting your FLOW 8

1. Connect the FLOW 8 to your computer via USB.
2. Power on the mixer.
3. Launch the application.
4. The app automatically detects the FLOW 8 and connects to it.

### 2.3 Device Selection Screen

If the FLOW 8 is not detected, the app shows the Device Selection screen with an error message.

| Action | Description |
|--------|-------------|
| **Retry** | Scans for MIDI devices again |
| **Copy Log** | Copies the debug log to clipboard |
| **Save Log** | Saves the debug log to a file |

> **Tip:** If the FLOW 8 is not detected after confirming the USB cable is connected and restarting your computer, use **Copy Log** or **Save Log** to capture the debug log and [open an issue on GitHub](https://github.com/abelroes/flow-8-midi/issues) with the log attached.

***

## 3. Navigation Bar

The navigation bar is always visible at the top of the application once connected.

* **Mixer** — the main mixing view.
* **Gear icon (⚙)** — opens Settings.
* **Question mark (?)** — opens the About page (app info, manual, debug log).
* **Sync button** — visible when Bluetooth is connected; requests the current state from the mixer.

### 3.1 Sync Status

| Indicator | Meaning |
|-----------|---------|
| 🟢 **Synced** | All UI parameters match the mixer's actual state |
| 🟡 **Unsynced** | Parameters have been changed in the UI but not yet confirmed by the mixer |

The last sync timestamp is displayed next to the indicator.

### 3.2 BLE Indicator

| Color | Status |
|-------|--------|
| 🔵 Blue | Connected |
| 🟡 Yellow (blinking) | Scanning / Connecting / Authenticating |
| 🔴 Red | Error |
| ⚫ Gray | Disconnected or unavailable |

* **Double-click** the BT indicator to connect or reconnect.
* The **Disconnect** button ends the BLE session.

***

## 4. Mixer Page

The main mixing view, in an SSL-style console layout: channel strips for all 7 input channels, with the bus strips (Main, Monitor 1/2, FX 1/2) to the right.

### 4.1 Input Channel Strips

Each input channel strip contains, from top to bottom:

| Control | Type | Availability |
|---------|------|--------------|
| **Channel label** | Text (e.g., "Ch 1 · XLR", "USB/BT") | All channels |
| **Channel name** | ✎ rename button | All channels |
| **Gain** | Knob (−20 to +60 dB) | Ch 1–6 only |
| **Low / LoMid / HiMid / High** | 4-band EQ knobs (center = flat) | All channels |
| **Cut / Comp** | Low Cut (20–600 Hz) and Compressor (0–100%) knobs | Ch 1–6 only |
| **Mon1 / Mon2 / FX1 / FX2** | Send level knobs | All channels |
| **Pan** | Knob (L–C–R) | All channels |
| **Level** | Fader (OFF / −70 to +10 dB) with VU meter | All channels |
| **Mute / Solo** | Toggle buttons (M and S) | All channels |
| **48V** | Phantom power toggle | Ch 1–2 only (XLR) |

> **Note:** Channel 7 (USB/BT) does not have Gain, Cut, or Comp controls — this matches the physical mixer's behavior.

> **Note:** Phantom Power (48V) requires a **double-click** to toggle, preventing accidental activation. The first click shows a warning; the second click confirms.

> **Note:** The channel name (✎) is sent to the mixer over BLE. Names are not read back from the device — after reconnecting, the field is empty again until you rename.

### 4.2 Bus Strips

Main, Monitor 1, and Monitor 2 each have:

| Control | Type |
|---------|------|
| **Level** | Fader with VU meter |
| **Bal** | Balance knob (Main only) |
| **62 Hz – 16 kHz** | 9-band graphic EQ knobs (62, 125, 250, 500, 1k, 2k, 4k, 8k, 16k) |
| **Limiter** | Knob |

### 4.3 FX Strips

FX 1 and FX 2 are the last two strips. Each contains:

| Control | Type | Description |
|---------|------|-------------|
| **Preset** | Dropdown | Selects the FX algorithm. |
| **Param 1** | Knob | Labeled dynamically per preset (e.g., "Decay" for reverbs, "Feedback" for delays) |
| **Param 2** | Toggle (A/B) | Labeled dynamically per preset (e.g., "Instrument/Vocal", "Dull/Bright", "Mono/Stereo") |
| **TAP** | Tap tempo button with 7-segment readout (FX 1 strip only) | Tap repeatedly to set the FX tempo; affects delay/echo presets. |
| **Main / Mon1 / Mon2** | Routing checkboxes (one row) | Where the FX return is routed. |
| **Level** | Fader | The FX bus return level. |

***

## 5. Settings Page

Opens with the gear icon. Three cards:

### 5.1 FLOW 8 Device

The mixer's back-panel and routing settings. Clicking an option sends it to the mixer over BLE; the **blue highlight** shows the setting the mixer reported at connect (read back from the device's state dump).

| Setting | Options | Description |
|---------|---------|-------------|
| **Footswitch** | FX mute / Recall snapshot | What the connected footswitch does |
| **Phones only** | Main + phones / Phones only | Mutes MAIN and plays everything on the phones out |
| **Phones mix** | Main mix / Mon 1/2 mix | What the phones output plays |
| **Phones tap point** | Pre-fader / Post-fader | Only affects the Mon 1/2 phones mix |
| **USB streaming** | 2-channel / 4-channel | How many channels the computer sends back (plays on Ch 5/6 and 7/8) |
| **Ch 5/6 · 7/8 audio** | BT audio / USB player | What the BT/USB channel plays |
| **Mon 1/2 source** | Monitor buses / USB 1/2 / USB 3/4 | What the MONITOR 1/2 outputs carry |
| **Mon link** | Unlinked / Linked | Links MONITOR 1 and 2 into one stereo output |
| **Ch 5/6 source** | LINE / USB | What channel strip 5/6 gets |
| **Ch 7/8 source** | LINE / USB | What channel strip 7/8 gets |
| **Main pad** | Off / −10 dBV | Attenuates the MAIN XLR outputs |
| **Monitor pad** | Off / −10 dBV | Attenuates the MONITOR outputs |

### 5.2 Snapshots

A compact grid of the 15 mixer snapshot slots (5 columns). Snapshot names are loaded via Bluetooth during connection.

| Element | Description |
|---------|-------------|
| **Named slot** (highlighted) | A saved snapshot — click to load |
| **Empty slot** (dimmed) | No snapshot saved in this slot |
| **✎** | Rename the snapshot (named slots only) |
| **Save** | Save the current mixer state into this slot |
| **Del** | Delete the snapshot (asks for a second click to confirm) |
| **Reset to Default** | Resets the mixer to its default state |

> **Note:** Loading a snapshot recalls the saved preset on the mixer and triggers a resync after a short delay to update the UI. Loading works over USB — BLE is not required.

### 5.3 Appearance & Sync

| Setting | Description |
|---------|-------------|
| **Theme** | Switch between the iced built-in themes |
| **Sync Interval** | How often the app automatically syncs with the mixer via Bluetooth |

***

## 6. Bluetooth (BLE) Sync

### 6.1 Why Bluetooth?

The FLOW 8's USB connection is **one-way**: the app can send commands to the mixer, but the mixer does not report its current state back over USB. Bluetooth solves this. When connected via BLE, the app asks the mixer for a full dump of its current parameters, and the mixer responds with the state the app uses to update the UI — the same mechanism the official Behringer mobile app uses.

**In short:**

* **USB only** — You can control the mixer, but the app starts "blind" (doesn't know the mixer's current state).
* **USB + Bluetooth** — You can control the mixer AND see its current state reflected in the UI.

### 6.2 What It Does

Once Bluetooth is connected, the app can:

1. **Read the mixer's current state** — fader positions, EQ settings, sends, FX, mute/solo states, and the device settings (Section 5.1) are fetched and reflected in the UI.
2. **Display snapshot names** — the names you assigned to your mixer snapshots are shown in the snapshot tiles.

> **Important:** The FLOW 8 only supports **one Bluetooth connection at a time**. Disconnect the official Behringer app before connecting with this application.

### 6.3 Auto-Sync

When BLE is connected and a sync interval is configured (Settings → Appearance & Sync → Sync Interval), the app periodically requests the mixer's state and updates the UI to stay in sync.

### 6.4 Manual Sync

Click the **Sync** button in the navigation bar to request an immediate state update from the mixer.

***

## 7. FAQ — Troubleshooting

**Q: Bluetooth won't connect / BLE indicator stays red.**

A: The FLOW 8 only allows **one Bluetooth connection at a time**. The most common cause is the official Behringer mobile app (Android/iOS) still being connected. Close the official app or disable Bluetooth on your phone, then try again.

If the official app is not running and BLE still won't connect, try:

1. Disconnect and reconnect the FLOW 8 USB cable.
2. Power-cycle the FLOW 8 (turn off, wait 5 seconds, turn on).
3. Restart your computer — some Bluetooth adapters require a fresh OS-level reset to release stale connections.

> **Tip:** After closing the app, wait ~30 seconds before reconnecting — the mixer holds the previous BLE connection for a while.

***

**Q: The app says "FLOW 8 not found" even though the USB cable is plugged in.**

A: Check the following:

1. Make sure the FLOW 8 is **powered on** (the mixer must be running, not just physically plugged in).
2. Try a different USB cable or USB port — some cables are charge-only and don't carry data.
3. On Linux, make sure your user has permission to access MIDI devices (you may need to be in the `audio` group).
4. Restart your computer — some USB MIDI drivers require a fresh start after the mixer is connected for the first time.

If nothing works, use **Copy Log** or **Save Log** on the Device Selection screen and [open an issue](https://github.com/abelroes/flow-8-midi/issues) with the log attached.

***

**Q: I moved a fader on the physical mixer, but the app didn't update.**

A: This is expected behavior. USB MIDI to the FLOW 8 is **one-way** — the mixer does not report physical control changes back over USB. To update the UI with the mixer's current state, connect via Bluetooth and use **Sync** (or enable Auto-Sync). See [Section 6](#6-bluetooth-ble-sync).

***

**Q: The app was working but now nothing responds when I move sliders.**

A: The USB connection may have dropped. This can happen if the USB cable is loose or the mixer entered a low-power state. Try:

1. Check the USB cable connection on both ends.
2. Click **Retry** on the Device Selection screen (or restart the app).
3. Power-cycle the FLOW 8.

***

**Q: Tap Tempo doesn't seem to do anything.**

A: Tap Tempo only affects **delay/echo presets**. If the FX slot is set to a Flanger or Chorus preset, tapping will have no audible effect. Also, you need to tap **at least twice** — the mixer measures the interval between consecutive taps to calculate the BPM.

***

## 8. For Developers

* **[Developer Manual](./DEV_MANUAL.md)** — Debug tools, calibration workflow, CLI utilities, and how to build with dev-tools enabled.
* **[MIDI Implementation Research](./flow8-midi-implementation.md)** — Protocol reverse-engineering: USB MIDI, BLE authentication, SysEx dump structure and encoding.

***

*FLOW 8 MIDI Controller is free software licensed under [GNU GPLv3](../LICENSE).*
*Author: Ingo Pfennigstorf — [github.com/ipf/flow-8-midi](https://github.com/ipf/flow-8-midi). Original by Abel Rocha Espinosa — [github.com/abelroes](https://github.com/abelroes).*

***

> ☕ **Enjoying FLOW 8 MIDI Controller?** This project is developed and maintained in my spare time. If you find it useful, consider [buying me a coffee](https://buymeacoffee.com/abelroes) — it keeps the project alive!
