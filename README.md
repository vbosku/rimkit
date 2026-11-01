# RimKit

A lightning-fast, native RimWorld mod manager built in **Rust** and **egui**.

---

## Features

- **High-Performance UI**: Smooth multi-select (`Shift` / `Ctrl` clicks), drag-and-drop support, instant filtering, and a full undo/redo stack (`Ctrl+Z` / `Ctrl+Y`).
- **Robust Dependency Resolution**: Stable topological sorting handling `loadAfter`, `loadBefore`, dependencies, Harmony, Core, and official DLCs.
- **Safe Atomicity**: Modlist writes use atomic file replacement (temp file + rename) to protect your `ModsConfig.xml`.
- **Automated Backups**: Timestamped backups before every save, keeping the 20 most recent states while discarding identical copies.
- **Advanced Diagnostics**:
  - Real-time issues banner (missing/duplicate mods, broken dependencies, incompatibilities).
  - Native log parser for `Player.log` with repeat grouping and stack-frame mod attribution.
  - VRAM estimation and texture cache monitoring per mod.
- **Cross-Platform & Seamless**: Direct Steam integration for launching the game and automated workspace recovery.

---

## Architecture (MVC)

| Layer | Path | Role |
|---|---|---|
| **Model** | `src/model/` | Pure logic and disk operations (`mod_info`, `mods_config`, `sort`, `issues`, `logs`, `startup`, `textures`, `store`). UI-agnostic. |
| **Controller** | `src/controller.rs` | State management. Single entry point via `Controller::apply(Action)` and background task polling (`src/task.rs`). |
| **View** | `src/view/` | Built with `egui`. Renders state, emits `Action`s, and relies on centralized styling tokens (`view/style.rs`). |

---

## Building & Testing

Ensure you have a stable Rust toolchain installed.

```bash
# Build in release mode
cargo build --release

# Run unit tests
cargo test

# Run read-only integration checks against real local installs and logs
cargo test --release -- --ignored --nocapture
```

## Performance Diagnostics

You can measure startup times by setting the environment variable:

```Bash

RIMKIT_TIMING=timing.log cargo run --release
```

Each launch will append `first_frame_ms` and `mods_loaded_ms` metrics to the specified file.

## License

This project is licensed under the GPL-3.0-only license. See the LICENSE file for details.
