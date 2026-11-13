# flow-8-midi — Design Document

A Linux desktop app (Rust + iced) that flashes and controls the Behringer
FLOW 8 digital mixer natively over Bluetooth LE, with a mixer-style UI.
The official control app is Android/iOS only; this app reverse-engineers
its BLE protocol and re-implements it, using USB MIDI as a secondary
control surface.

## Architecture

Elm-architecture (iced 0.14): a single `update()` folds messages into the
model, `view()` renders from the model, services produce messages.

```
src/
  main.rs          update(), view dispatch, BLE-driven update thread glue
  model/
    flow8.rs       FLOW8Controller — the one model: channels, buses,
                   fx_slots, snapshots, BLE status, meter state
    channels.rs    Channel, Bus, BusType, FxSlot, FxPresetInfo, preset tables
    message.rs     InterfaceMessage — every UI/service event
    page.rs        Page enum
  service/
    ble.rs         BLE connect/auth/session, command TX, meter loop,
                   dB tables (F48), fix8 conversions
    midi.rs        USB MIDI device discovery
    midi_mapper.rs InterfaceMessage → USB CC (the MIDI surface)
    sysex_parser.rs  parses 0x38 state dumps into the model
    sysex_calibration.rs
  view/
    mixer_page.rs  THE page: channel strips + bus strips (incl. FX strips)
    fx_page.rs     compact FX widget builders (imported by the mixer)
    widgets.rs     knob, v_slider, sync dots/labels, formatters
    nav_bar.rs     Mixer-only nav (Eq/Sends/Fx pages are gone)
    *_page.rs      DeviceSelect, Settings, Snapshots (Eq/Sends kept but
                   unreachable)
  bin/            dev harnesses: test_cm (BLE write tests), calibrate,
                   analyze_names, digest
```

### Data flow

1. UI event → `InterfaceMessage` → `update()` mutates the model and
   triggers a service action (BLE write in a worker thread, serialized
   on the connection mutex).
2. BLE notifications (meter frames, state dumps) arrive on background
   threads → messages/apply_dump_to_controller → model → view.
3. `midi_mapper` mirrors relevant messages to USB MIDI CC (channels 1..,
   bus ids 10/11 on CC#7).

## BLE protocol (reverse-engineered)

Oracle: `../FLow8` (Kotlin rebuild of the official app). Native dumps in
`/tmp/opencode/*.cap` (amidi captures, 3068-byte full dumps).

### Session

- Characteristic: single GATT characteristic for everything.
- Handshake: device sends 0x35 challenge (24 B), we reply with the auth
  key, device confirms with 0x36. If 0x36 doesn't arrive: the device's
  session table is wedged — wait 60-150 s or power-cycle.
- ~40 s stale-session delay after killing the app.
- Writes that change nothing only move dump header counter bytes
  (0x0C-0x18 = per-dump sequence/checksum, not state).

### Commands (queued-chunk framing: `[len][type][payload...]` unless noted)

| Type | Name          | Wire payload                                            |
|------|---------------|---------------------------------------------------------|
| 0x06 | ChannelMaster | `[id][dst][fix8 dB]` — THE fader path, all outputs      |
| 0x07 | ConfigRequest | raw 3 B `[0x07][0x01][0x08]` (triggers 0x38 dump)       |
| 0x08 | ChannelMute   | `[id][flag]` — inputs AND FX buses (FX mute = 0x0C/0x0D)|
| 0x11 | FXSetup       | `[fx_id][p1][p2][p3][main][mon1][mon2]` routing flags   |
| 0x21 | MeterRequest  | raw single form `[0x21][0x01][out][byte-sum]`           |
| 0x30 | FXChannelState| NOT sent by the official app; dead end (deleted)        |
| 0x31 | FXLoadPreset  | `[slot][preset]`                                        |
| 0x35/36 | Handshake  | challenge / auth reply                                  |
| 0x38 | State dump    | 4 × 128 B chunks; channel on/flags, EQ, FX presets      |

- **fix8**: dB values are F48-table indexes ("selector 3"), not linear
  bytes. UI CC scale is −70…+10 dB (cc 0 = OFF/−144, 0 dB = cc 111);
  `level_cc_to_db` / `db_to_fix8` / `fix8_to_level_cc` in ble.rs.
- **channelId space** (device): Main=0x0F, Mon1/2=0x0A/0x0B,
  FX1/2=0x0C/0x0D. Our bus ids 10/11 → `0x0A + (bus_id − 8)`.
- **Metering**: MeterRequest output byte = `channelId | 0x40 | 0x80`
  (Main=0xCF, Mon=0xD8/0xD9, FX1=0xCC). Meter update frames are 20 B:
  `[0x16][?][15 × fix8 dB][GR…]`. Meter loop polls one output at a time,
  advances only on frame land (poll↔frame 1:1), `FLOW8_METER_OUT` env
  pins one output for debugging.

### Ground truths (learned the hard way)

1. **FX chain is send → engine → routing → Main.** If FX routing to Main
   is off, nothing downstream is audible. The FX bus meter is
   **post-routing**, so a working return still meters silence — early
   "dead return write" proofs were confounded by this.
2. **ChannelMaster (0x06) is the official path for every output fader**,
   including FX returns (`dst == id` sets the bus's own fader; `dst =
   0x0C/0x0D` sets a send).
3. **Preset loads wipe FX state** (routing + params + returns). After
   each FxPreset the app re-asserts FXSetup (0x11) + ChannelMaster
   returns after 600 ms.
4. **The 0x38 dump does not expose FX channel mute/level/routing** —
   only FX preset/params bytes update reliably. The meter is ground
   truth for FX signal flow; dump diffs prove writes dead only for what
   the dump contains.
5. **0x30/FXChannelState is a dead end**: the official app never sends
   it (no queueFXChannelState in the native dump), and our rebuilt
   writes either no-op'd or landed shifted. All 0x30 helpers deleted.
6. FX mute = ChannelMute (0x08) on 0x0C/0x0D (official
   `queueUpdateFXChannelMutesCommandPackets` sends exactly that).

## UI

One page: the mixer. (EQ/Sends/FX pages removed — the mixer strips cover
everything: 4-band EQ per input, 9-band per bus, fx send knobs, FX
returns, and the FX controls.)

- **Channel strips** (mic 1-4, line/USB/BT…): name (inline rename),
  phantom, 4-band EQ knobs, sends (FX1/FX2), pan, M/S buttons, fader +
  meter + dB label, right-click = zero.
- **Bus strips** (Mon 1, Mon 2, FX 1, FX 2, Main — group breaks between
  Mon|FX|Main):
  - Mon/Main: 9-band EQ (2 knobs/row) + Pan + Limiter, block floating
    vertically centered between label and fader; fader + meter at the
    bottom, aligned with channel faders.
  - FX strips host the whole FX control set: M button (global FX mute,
    channel-strip style, TAP tempo next to FX1's M), preset dropdown
    (pick_list), param1 knob (intensity, sends FXSetup on change),
    param2 dropdown (Instrument/Vocal, Wide/Narrow…), routing checkboxes
    (Main/Mon1/Mon2, stacked), return fader.
- **FX state model**: `FxSlot { preset, param1-3, route_main/mon1/mon2,
  *_synced }`. `preset` is 0-based; the wire/dump raw byte is preset+1.
- Default window 1500×700 (screen-override via `FLOW8_SCREEN=WxH`).

## MIDI mapping (USB CC)

- Channel n: CC#1/2/3/4 = EQ, #5 mute, #6 solo, #7 level.
- BusLevel → CC#7 on channel = bus id (FX returns on 10/11).
- SendFx1/2 → CC#16/17 (proven path for fx sends).

## Known limitations / parked

- FX sends via ChannelMaster dst=0x0C/0x0D unverified (USB CC is the
  working path).
- Device-state readback of FX routing/mute not parsed (0x38 MixerState
  chunks logged hex at connect; FX1 = slot 10 flags byte).
- BLE auth wedges after ~15 sessions/day — needs power-cycle; no
  reconnect-on-wedge logic.
- Eq/Sends page modules still compiled but unreachable from nav
  (candidates for deletion).
- Parked false trails (do not reopen without new evidence): FX return
  via 0x30, FXChannelState compact wire form, 0x41/0x42/0x43 bulk
  family, tempo 0x40-vs-0x3F, clipboard-copy bug, snapshot-refresh
  second-subscribe stall, 0x25 snapshot highlight.

## Dev workflow

- `cargo build --release`; run `target/release/flow-8-midi` with
  `DISPLAY=:0 WAYLAND_DISPLAY=wayland-0`.
- BLE write harness: `cargo run --release --bin test_cm` (while the app
  is closed; amidi can't open the device while the app runs).
- Meter pinning: `FLOW8_METER_OUT=CC` (hex output id) for FX1 bus.
- Log: `/tmp/opencode/flow8.log`.
