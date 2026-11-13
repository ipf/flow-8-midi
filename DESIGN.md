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

### UI design decisions

1. **Mixer is the only page.** Every control lives in a strip; the nav
   bar is Mixer + a gear (Settings). Rationale: the old Eq/Sends/Fx
   pages duplicated strip content (user directive: "Mixer is only
   relevant").
2. **Faders bottom-aligned across all strips** (channel + bus): every
   strip ends with Level label + fader + meter + dB text, pushed to the
   bottom by a Fill spacer, so fader caps form one visual line.
3. **Bus row grouped Mon | FX | Main** with 3px group breaks (the same
   break as between the channel and bus sections), 2px between strips.
4. **Channel strip order** (top to bottom): name (inline rename, ✕
   cancel) → M/S buttons → phantom → 4-band EQ knobs → FX sends → pan →
   fader. Mirrors the official app's channel layout.
5. **Mon/Main bus strips**: 9-band EQ at 2 knobs per row (3-per-row
   overflowed the ~117px strip), then Pan and Limiter each on their own
   row, Limiter directly above Level. The whole EQ + pan/limiter block
   floats **vertically centered** between the strip label and the fader
   (two Fill spacers share the leftover space) — everything clustered at
   the top left a hole below.
6. **FX strips host the full FX control set** (the old Fx page is gone):
   - M button (global FX mute, channel-strip danger style, shown in
     both strips since the state is one global flag), TAP tempo button
     next to FX1's M (tap targets FX2 delay; global controls are kept
     compact by living in FX1's strip)
   - preset dropdown (pick_list; model preset is 0-based, wire byte is
     preset+1)
   - param1 knob (intensity/modulation, percent 0-100; sends FXSetup
     on every change — knobs have no on-release)
   - param2 dropdown (Instrument/Vocal, Wide/Narrow, Dull/Bright…)
   - routing checkboxes stacked vertically (Main/Mon1/Mon2) — the FX
     master switch that caused the original outage
   - return fader (ChannelMaster 0x06 path)
7. **FX strips stay top-anchored** (M at the top like channel M/S);
   only Mon/Main blocks are centered.
8. **Right-click = zero** on sliders (faders, sends); knobs reset via
   drag to their default (knob canvas has no right-click).
9. **Sync dots**: every adjustable value shows an unsynced dot (●) in
   `UNSYNCED_COLOR` until the device echoes it (knobs, sliders, preset
   label). Dropdowns/checkboxes show no dot (see inconsistencies).
10. **Meters**: thin always-visible VU bars (dark track, lit portion);
    GR slot when present. The FX bus meter is post-routing — it shows
    signal only when FX routing is enabled.
11. **Uniform control sizes**: all knobs 28px (channel EQ, bus EQ, FX
    param1), sliders 150px (channels) / 150px buses, one label scale
    (strip label 14, control labels 10-12, dB text 10).
12. **Spacing scale**: strip internal spacing 2, group breaks 3,
    padding [6,3]; window default 1500×700 (`FLOW8_SCREEN=WxH` override).
13. **FX state model single source**: `FxSlot` in the model; every FX
    control writes the model first, then a service action — view never
    holds state.

### UI inconsistencies & open proposals

1. **Eq/Sends pages are dead code.** `view_eq`/`view_sends` match arms
   are unreachable (nav = Mixer only), and they carry duplicated FX
   fader logic (`TABS`, `FX_BUS_FADER_HEIGHT`). Transitively dead:
   `MonitorTab` message + `monitor_tab` model field + its midi_mapper
   arm. Proposal: delete both modules + the dead message/field
   (~500 lines, zero runtime cost).
2. **`fx_page.rs` is no longer a page** — it only holds the compact FX
   widget builders. Proposal: rename to `fx_widgets.rs` (mechanical).
3. **`PresetChoice` and `ParamChoice` are duplicate wrappers** in
   fx_page.rs (both `{value/idx, name}` + Display). Proposal: one
   shared `Choice { value: u8, name: &'static str }`.
4. **Sync feedback is inconsistent**: routing checkboxes and the param2
   dropdown show no unsynced dot (model has no route/param2-sync flags).
   Bounded risk: preset loads re-assert routing+params, so drift is
   rare. Proposal (optional): add `*_synced` flags for routing.
5. **M button appears in both FX strips** (one global flag shown
   twice) while TAP lives only in FX1. Proposal: keep (state visibility
   beats asymmetry) or move M to FX1 only for the strict
   "global controls in FX1" rule.
6. **Bus fader paths differ**: FX returns go BLE ChannelMaster;
   Mon/Main bus faders still go USB CC#7 (`channel = bus_id`) — that
   path is unverified against the meter. Proposal: verify Mon/Main
   faders audibly; if dead, move all bus faders to ChannelMaster
   (official path for every output).
7. **Label sizes differ**: channel name 11 (button) vs bus label 14.
   Proposal: unify to 13.

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
