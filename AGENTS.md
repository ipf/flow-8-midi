# AGENTS.md

Rules for AI agents working in this repository.

## Manual

* Update `docs/MANUAL.md` with **every** UI or feature change — new/removed controls, renamed pages, changed behavior. The manual is compiled into the binary (`include_str!` in `src/view/manual_page.rs`) and shown in the app's Manual page, so it must match the running app.
* Update `docs/DEV_MANUAL.md` with every protocol change (BLE framing, USB SysEx, opcodes).
* Keep the manual render-friendly: standard markdown (headings, bullets, tables, blockquotes) — the in-app viewer applies light formatting only (heading sizes, bullets, separators; inline markers are stripped).

## Device verification

* Protocol work must be verified on the real device before committing; the oracle for the official protocol is the Kotlin rebuild in `../FLow8`.
* The mixer stays natively flashable/controllable from this app — never break that.
