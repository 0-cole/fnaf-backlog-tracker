# FNAF Backlog Tracker (`fnaf-backlog-tracker`) Context

## Executive Summary & High-Level Objective
Native Windows desktop GUI built in Rust (`eframe` / `egui` 0.29 with `glow`) to manage a curated 84-game FNAF fangame backlog with a reorderable **Priority Queue**, unmistakable non-text visual status indicators (**Played**, **Priority**, **To-Do**, **Dropped**), 1-click browser lookup links, and JSON persistence. Also includes a companion Syncthing transfer bundle (`C:\Users\cdbla\Documents\Antigravity Playground\FNAF-Transfer`) containing *Freddy's: Reanimated* (`freddys-r-v1-0-2.exe`) and a per-PC hardware-ID save generator (`restore_save.ps1` -> Night 3, Name `Cole`).

---

## Critical Architecture & Invariants
1. **Rust Toolchain & Custom Linker Setup (`coleslaptop`)**:
   - `cargo`/`rustc` (`1.99.1`) live in `%USERPROFILE%\.cargo\bin` (not in default PATH).
   - Visual Studio 18 (`2026` Community) had `link.exe`, `dumpbin.exe`, and `lib.exe` at `C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.51.36231\bin\Hostx64\x64`, but lacked Windows 10 SDK `.lib` files (`NETFXSDK` only).
   - `.cargo\config.toml` points `linker` to VS 18 `link.exe` and links against synthesized System32 Win32/UCRT import libraries in `winlibs\` (`30` `.lib` files including `msimg32.lib` and `winspool.lib`). Keep `.cargo\config.toml` and `winlibs\` intact when rebuilding on `coleslaptop`.
2. **GUI & Persistence (`src/main.rs`, `src/games.rs`)**:
   - Pure vector icons drawn via `egui::Painter` (checkmarks, stars, X circles, left accent stripes, segmented progress bar) to avoid missing-glyph tofu boxes.
   - Browser links launch cleanly via `rundll32 url.dll,FileProtocolHandler <url>` with `CREATE_NO_WINDOW`.
   - State saves/loads automatically to `fnaf_backlog_state.json` next to `fnaf-backlog-tracker.exe`.
3. **Syncthing PC Handoff (`FNAF-Transfer`)**:
   - Transfer directory: `C:\Users\cdbla\Documents\Antigravity Playground\FNAF-Transfer`
   - *Freddy's: Reanimated* (`Damned Esta`) encrypts `main.cfg` using `OS.get_unique_id()` (`GetCurrentHwProfileW`); `restore_save.ps1` re-encrypts Night 3 / `"Cole"` on the target PC using pure .NET AES-ECB CFB-128.

---

## Recent Modifications & Verified State
- `Cargo.toml`, `src/games.rs`, `src/main.rs` compiled cleanly in release mode (`0` warnings, `0` errors) -> `target\release\fnaf-backlog-tracker.exe` (4,857,856 bytes).
- Debug trace verified `STEP1: main start`, `STEP2: BacklogApp::new`, `STEP3: first update()`.
- `restore_save.ps1` generated `main.cfg` (220 bytes) and `settingsfile.cfg` (140 bytes) locally and passed `Cryptodome` AES-256-CFB-128 decryption + MD5 validation.
- Syncthing folder `antigravity-playground` scanned (`state=idle`, `errors=0`), awaiting Desktop PC (`C5NQTSZ`) sharing acceptance for `coleslaptop` (`SW23URD`).

---

## Immediate Next Steps
- On Desktop PC (`DESKTOP-I98GCSO`), complete Syncthing pairing/sharing for `antigravity-playground` with `coleslaptop` (`SW23URD-TD4OGPK-HWNZKW7-4GWZR6Q-EZW6LJH-JY6TD7T-3OV76NX-VAJQMAB`) and run `FNAF-Transfer\setup_on_pc.bat`.
- Whenever future edits are made to the Rust app on `coleslaptop`, rebuild via `cargo build --release` (with `%USERPROFILE%\.cargo\bin` on PATH) and copy the updated `fnaf-backlog-tracker.exe` to `FNAF-Transfer\`.
