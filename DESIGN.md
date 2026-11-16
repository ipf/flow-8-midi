# flow-8-midi — UI Design

The UI design of the FLOW 8 MIDI Controller desktop app (Rust + iced):
an SSL-style, one-screen mixer for the Behringer FLOW 8. This document
covers UI only; protocol and architecture live in the code and
`docs/DEV_MANUAL.md`.

## Pages

Nav bar (top): Mixer tab, BT indicator + Disconnect (when connected),
Settings gear, Help button. Four pages plus an error state:

* **Mixer** — the main screen; every control lives here (see below).
* **Settings** (gear) — three cards: FLOW 8 Device (12 back-panel
  settings; the blue highlight shows the state the mixer reported at
  connect), Snapshots (15 slots in 5 columns: load / ✎ rename / Save /
  Del, plus Reset to Default), Appearance & Sync (theme, sync interval).
* **Manual** (About → Open Manual) — `docs/MANUAL.md` rendered in-app
  with light markdown formatting (heading sizes, bullets, grey quotes,
  ← Back). No browser.
* **About** (?) — app info, Open Manual, debug log tools with a
  "Show debug entries" switch (off by default: the log list stays
  hidden until enabled; Copy/Save exports always include everything).
* **Device Select** — error state when no FLOW 8 is found: message +
  Retry / Copy Log / Save Log.

## Mixer layout

12 strips in one row: 7 input channels (Ch 1–6, USB/BT), Mon 1, Mon 2,
FX 1, FX 2, Main. 2px between strips, 3px group breaks around the bus
section. One vertical rhythm across all strips: strip label at the top,
controls in the middle, level fader block bottom-aligned on every strip.

### Channel strip (top to bottom)

1. Strip label ("Ch 1 · XLR", 14px)
2. M / S toggle buttons; phantom 48V slot below (Ch 1–2 XLR only,
   double-click to confirm)
3. Gain knob (Ch 1–6)
4. 4-band EQ: Low / LoMid / HiMid / High knobs, 2 per row
5. Cut / Comp knobs (Ch 1–6)
6. Sends: Mon1 / Mon2, FX1 / FX2 knobs, 2 per row
7. Pan knob
8. Level fader block: "Level" label, clip dot, 150px fader with VU bar,
   dB text (OFF / −70…+10 dB)

### Bus strips (Mon 1 / Mon 2)

Strip label → 9-band graphic EQ (62 Hz–16 kHz, 2 knobs per row) →
Limiter knob → Level fader block. The EQ + limiter block floats
vertically centered between the label and the fader (two Fill spacers
share the leftover space) — clustering at the top left a hole below.

### FX strips (FX 1 / FX 2)

Strip label → M button (FX1 only, global FX mute, channel-strip danger
style) → Preset dropdown → Param 1 knob (percent, label per preset) →
Param 2 dropdown (A/B, label per preset) → TAP block (FX1 only) →
routing band → Level fader block.

* **TAP block**: 7-segment LED readout above the button — red digits on
  a near-black rounded panel, fixed 64px wide (`---` before the first
  tap), DSEG7 Classic font (bundled, OFL). Shows the tapped tempo
  (5-tap average).
* **Routing band**: Main / Mon1 / Mon2 checkboxes in one horizontal
  row, labels above their checkboxes, bottom-anchored in the pan/
  limiter band slot.

### Band alignment

Every strip's bottom band — channel Pan, bus Limiter/Pan, FX routing —
sits directly above the fader block: one horizontal band line across
the whole mixer.

## Controls & interaction

* **Knobs**: 28px canvas widgets, drag to adjust; drag to the default
  position resets. Tooltips show the value in real units (dB, Hz, %).
* **Sliders/faders**: 150px, right-click = zero.
* **Sync feedback**: an adjustable value shows a small dot (●) while
  unsynced (changed in the UI, not yet echoed by the device).
* **Meters**: thin always-visible VU bars (dark track, lit portion);
  GR meter slot where present. The FX bus meter is post-routing — it
  shows signal only when FX routing is enabled.
* **Rename**: channel and snapshot names edit inline (✎ → text field,
  ✕ cancels); snapshot Save prefills the name field.

## Styling

* Dark theme (iced dark base).
* Uniform control sizes: all knobs 28px, faders 150px.
* Label scale: strip label 14, control labels 9–12, dB text 10.
* Spacing: 2px inside strips, 3px group breaks, `[6, 3]` padding.
* Colors: unsynced dot orange (1.0, 0.7, 0.15); mute/danger red
  (0.9, 0.2, 0.2); TAP readout red (1.0, 0.12, 0.08) on near-black
  (0.06, 0.02, 0.02).
* Window: preferred 1500×700 (`FLOW8_SCREEN=WxH` override), min
  800×500.
* Fonts: default iced sans; DSEG7 Classic (bundled, OFL) for the TAP
  readout.
