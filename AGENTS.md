# AGENTS.md

Rules for AI agents working in this repository.

## Documentation — ALWAYS update with every change

Every change keeps these documents in sync, in the same commit:

* `README.md` — features, how-to, download, screenshots
* `AGENTS.md` — this rule list
* `DESIGN.md` — UI design only (pages, strips, controls, styling)
* `docs/MANUAL.md` — user manual; compiled into the binary
  (`include_str!` in `src/view/manual_page.rs`) and shown in the app's
  Manual page, so it must match the running app
* `docs/DEV_MANUAL.md` — protocol changes (BLE framing, USB SysEx,
  opcodes)

Keep `docs/MANUAL.md` render-friendly: standard markdown (headings,
bullets, tables, blockquotes) — the in-app viewer applies light
formatting only (heading sizes, bullets, separators; inline markers are
stripped). Keep `DESIGN.md` free of protocol/architecture content.

## Device verification

* Protocol work must be verified on the real device before committing;
  the oracle for the official protocol is the Kotlin rebuild in
  `../FLow8`.
* The mixer stays natively flashable/controllable from this app — never
  break that.
