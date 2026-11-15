<div align="center">

<img src="app-icon.svg" alt="DiskSieve logo" width="96" height="96" />

# DiskSieve

**A fast, offline disk space analyzer for developers and power users.**

It answers not only *“what is taking up space?”*, but also *“what can I safely delete?”* and *“what has grown since last time?”*.

[![CI](https://github.com/DiskSieve/disksieve/actions/workflows/ci.yml/badge.svg)](https://github.com/DiskSieve/disksieve/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-22C55E.svg)](LICENSE)
![Status](https://img.shields.io/badge/status-pre--alpha-F5B83D.svg)
![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20Linux%20%7C%20macOS-3E5A4B.svg)

</div>

> [!WARNING]
> DiskSieve is in early development (working towards `v0.1.0`). There are no installers yet — build it from source to try it.
> Scanning is read-only. The only action that changes your disk is **Move to Trash**, which always asks first, is checked by the backend and can be undone on Windows and Linux.

---

## Table of contents

- [Why DiskSieve?](#why-disksieve)
- [Features](#features)
- [Project status](#project-status)
- [Roadmap](#roadmap)
- [Getting started](#getting-started)
- [Architecture](#architecture)
- [Safety and privacy](#safety-and-privacy)
- [Performance targets](#performance-targets)
- [Contributing](#contributing)
- [License](#license)

## Why DiskSieve?

Existing tools show *what* takes up space, then leave you alone with the harder questions:

1. **“Can I delete this?”** — size alone does not tell a rebuildable cache from a critical system file.
2. **“What grew?”** — disks fill up gradually, and a one-off scan shows no trend.
3. **“How much space do my projects waste?”** — `node_modules`, `target/` and `.venv` in projects untouched for a year, plus global tool caches (npm, Cargo, Gradle, Docker), often add up to tens of gigabytes.

**DiskSieve aims for WizTree-class speed, DaisyDisk-class UX, and the decision context nobody else offers — on every OS, open source and fully offline.**

| Tool | Strengths | What DiskSieve adds |
| :--- | :--- | :--- |
| WizTree | Fastest scan on Windows (MFT) | Cross-platform, open source, safety advice, history |
| WinDirStat / SpaceSniffer | Classic treemap | Faster multi-threaded scan, advice, delta |
| DaisyDisk | Beautiful UX, deletion basket | Free, Windows and Linux |
| ncdu / dust | Fast, terminal | GUI, safety verdicts |
| npkill / kondo | Project artifact cleanup | The whole disk in context, not only artifacts |
| Czkawka | Duplicates and cleaners | Treemap and disk structure analysis |

## Features

✅ available now · 🚧 in progress · 🗓️ planned

| | Feature | Description |
| :---: | :--- | :--- |
| ✅ | **Multi-threaded scanner** | Parallel directory walker (`rayon`, one thread per logical CPU). Counts both logical size and size on disk, stays on one file system, never follows symlinks or junctions, and is cancellable at any time. |
| ✅ | **Live progress** | Files, folders, bytes and the current directory streamed ~10× per second. Permission errors never stop a scan — they are counted and reported. |
| ✅ | **Interactive treemap** | Drill down by clicking tiles or list rows, with breadcrumbs and an “up” button. Each view shows the 200 largest items plus one aggregated **Other** tile. |
| ✅ | **Size on disk vs. logical size** | One toggle switches between allocated space (clusters, compression, sparse files) and apparent size. |
| ✅ | **Color modes** | *By extension* (file categories) and *Cold Data* (age of the newest file in each folder). |
| ✅ | **Drive picker** | Mounted volumes with usage bars, or any single folder via the native picker. |
| ✅ | **App shell** | Custom title bar, startup “what to scan?” dialog, detailed scan progress dialog, dark loader instead of a white flash. |
| ✅ | **Space reconciliation** | “The disk reports 812 GB used, the scan sees 776 GB — here is where the other 36 GB are” (restore points, shadow copies, folders without access). |
| ✅ | **Safe Move to Trash** | System Trash only, protected paths enforced in the backend, each item re-checked against the scan before it is moved, instant tree update without rescanning, Undo on Windows and Linux. |
| ✅ | **File actions** | Right-click any tile or row: inspect, show in File Explorer / Finder, copy the full path, move to Trash. |
| 🗓️ | **Dev Junk** | Build artifacts detected by project marker files (no false hits on any random `build/` folder), grouped by project and its age. |
| 🗓️ | **Search (`Ctrl+K`)** | Instant name search in the scanned tree — no disk access. |
| 🗓️ | **Safety Advisor** | A verdict for every item (*safe* / *review* / *system*) with an explanation and a safe alternative (e.g. `powercfg -h off` instead of deleting `hiberfil.sys`). |
| 🗓️ | **Sieve Basket** | Collect items from every view, see the total space reclaimed and safety warnings, then trash them in one go. |
| 🗓️ | **Delta Mode** | Compare with previous scans: the treemap shows only what grew. |
| 🗓️ | **Duplicates** | Size → partial hash → full BLAKE3, skipping hard links and cloud-only files. |
| 🗓️ | **NTFS MFT engine** | WizTree-class scan speed on Windows (1 million files in under 3 s). |
| 🗓️ | **CLI and reports** | `disksieve scan <path> --json` for scripts and CI; CSV / JSON / Markdown export. |

## Project status

Working towards **`v0.1.0`** — Phases 1–3 are complete, Phase 4 is in progress.

| Phase | Scope | Status |
| :--- | :--- | :---: |
| **1. Foundations** | Cargo workspace, Tauri v2 shell, IPC types generated from Rust, CI on 3 OSes | ✅ |
| **2. Core** | Compact arena tree, multi-threaded scanner, viewport projection (Top-200 + Other) | ✅ |
| **3. MVP integration** | Live scan, treemap on real data, app shell, space reconciliation, safe file actions | ✅ |
| **4. Dev Junk and release** | Dev Junk engine and tab, `Ctrl+K` search, `v0.1.0` release | 🚧 |

<details>
<summary><b>Completed steps</b></summary>

- **1.1** Audit of the UI prototype and directory clean-up
- **1.2** Cargo workspace (shared metadata, dependencies, lints, release profile) and Tauri v2 skeleton with a strict CSP
- **1.3** IPC types generated from Rust with `ts-rs`, typed `invoke()` wrappers with a browser demo fallback
- **1.4** CI: `rustfmt`, `clippy -D warnings`, tests, generated-types freshness check and frontend build on Windows, Linux and macOS
- **2.1** Compact arena tree — 48 bytes per node, shared name buffer, `u32` indices, `mark_trashed` with ancestor propagation
- **2.2** Multi-threaded scanner — custom `rayon` walker, size on disk, hard links (Unix), cloud-only detection, cancellation, ~2.5 M entries/s on a warm cache
- **2.3** Viewport projection — Top-200 + Other with `select_nth_unstable`, size metric choice, breadcrumbs
- **3.1** Drive list, folder picker, cancellable scan with live progress
- **3.2** Treemap and list driven by `get_viewport` — drill-down, size metric toggle, stale-response rejection
- **3.3** App shell — custom title bar, startup scan dialog, scan progress dialog, instant dark first paint
- **3.4** Space reconciliation — “Unscanned / hidden” tile and an explanation panel with OS-specific causes and skipped paths
- **3.5** Safe file actions — backend delete guard, pre-trash verification, per-item results, Undo, reveal in file manager, copy path
- **4.1** Dev Junk engine — build artifacts confirmed by project marker files, grouped by project, project age that ignores the artifacts, nested artifacts counted once

</details>

## Roadmap

### `v0.1.0` — MVP: a fast, accurate and safe analyzer
- [x] Foundations: workspace, Tauri v2, generated IPC types, CI
- [x] Core: arena tree, multi-threaded scanner, viewport projection
- [x] Live scan with progress and cancellation
- [x] Treemap and list on real data
- [x] App shell: custom title bar, startup dialog, scan progress dialog, loader
- [x] Space reconciliation (“unscanned / hidden” tile with an explanation)
- [x] Safe file actions: Move to Trash with backend guard, reveal in file manager, copy path
- [ ] Dev Junk engine and cleanup tab
- [ ] `Ctrl+K` search
- [ ] Release: icons, installers for 3 OSes, portable `.exe`, `SHA256SUMS.txt`

### `v0.2.0` — Decision context
- [ ] Safety Advisor rules
- [ ] Sieve Basket
- [ ] Scan snapshots and Delta Mode
- [ ] Duplicate finder (BLAKE3)

### `v0.3.0` — Speed and accuracy
- [ ] NTFS MFT engine (Windows)
- [ ] Fast rescan from the USN Journal
- [ ] Cloud files (OneDrive, iCloud, Dropbox) in the UI
- [ ] Exclusions and “Run as administrator”
- [ ] Global developer caches and extension statistics

### `v0.4.0` — Integration and automation
- [ ] Settings (units, Cold Data thresholds, custom Dev Junk rules)
- [ ] “Scan with DiskSieve” in the system context menu
- [ ] CLI with JSON output
- [ ] Report export and snapshot history

### `v1.0.0` — Open-source readiness
- [ ] Community docs (`CONTRIBUTING`, `CHANGELOG`, `SECURITY`)
- [ ] Signed auto-updater
- [ ] Signed installers and accessibility

## Getting started

### Prerequisites

- **Rust** — recent stable toolchain (1.90 or newer) via [rustup](https://rustup.rs)
- **Node.js** 22+ and npm
- **Platform dependencies for Tauri v2** — see the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/):
  - Windows: Microsoft C++ Build Tools and WebView2 (preinstalled on Windows 10/11)
  - Linux: `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `build-essential`
  - macOS: Xcode Command Line Tools

### Run

```bash
git clone https://github.com/DiskSieve/disksieve.git
cd disksieve
npm install
npm run tauri dev
```

To work on the UI only, run `npm run dev` and open <http://localhost:1420>. Without the Tauri runtime the app uses **demo data**, so every screen works in a plain browser.

### Build a release binary

```bash
npm run tauri build
```

### Development commands

| Command | Purpose |
| :--- | :--- |
| `cargo test --workspace` | Run all Rust tests (file system tests use isolated temp directories) |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint |
| `cargo fmt --all` | Format Rust code |
| `cargo bench -p disksieve-core` | Scanner and viewport benchmarks (`criterion`) |
| `npm run gen:types` | Regenerate TypeScript types from Rust DTOs |
| `npm run build` | Type-check and build the frontend |

## Architecture

```text
disksieve/
├── crates/
│   ├── disksieve-core/   # Pure Rust library: scanner, arena tree, viewport projection, space balance,
│   │                     # delete guard, Dev Junk (later: duplicates, snapshots). No UI dependencies.
│   └── disksieve-cli/    # Terminal mode on the same core (placeholder until v0.4.0)
├── src-tauri/            # Thin desktop layer: IPC commands, app state, DTOs
└── src/                  # React 19 + TypeScript + Tailwind CSS v4 + shadcn/ui + ECharts
    └── types/generated/  # TypeScript types generated from Rust (never edited by hand)
```

```text
┌──────────────────────────┐   invoke() / Channel    ┌──────────────────────┐
│  React UI (WebView)      │ ──────────────────────▶ │  src-tauri commands  │
│  treemap, list, dialogs  │ ◀────────────────────── │  state, DTO mapping  │
└──────────────────────────┘  Top-200 + Other only   └──────────┬───────────┘
                                                                │
                                                     ┌──────────▼───────────┐
                                                     │   disksieve-core     │
                                                     │ scanner → ScanTree   │
                                                     │ build_viewport()     │
                                                     │ Guard → trash_nodes  │
                                                     └──────────────────────┘
```

Key design decisions:

- **Compact arena tree.** Nodes live in one flat `Vec<Node>` (48 bytes each), names in a shared buffer, links are `u32` indices. Target: **≤ 80 MB of RAM per 1 million files**.
- **Narrow IPC.** The full tree never leaves Rust. The UI asks for one folder at a time and gets at most 200 items plus one *Other* node.
- **Scan generations.** Every response carries the scan `generation`; answers from an older scan are rejected on both sides.
- **Generated types.** The TypeScript contract is produced from Rust with `ts-rs`, and CI fails if it is out of date.
- **Responsive by design.** Heavy commands run on blocking threads, and progress is throttled to ~10 updates per second.

## Safety and privacy

- **100% offline.** No telemetry, no accounts, no network connections (an optional update check will come in `v1.0.0`).
- **Read-only scanning.** The scanner reads metadata only and never opens files. Cloud-only files (OneDrive, iCloud) are detected by attributes and never downloaded.
- **Trash only.** Nothing is ever deleted permanently — items are moved to the system Trash after a confirmation. Volume roots, the home folder and its parents, system paths and files, DiskSieve itself, links and cloud-only files are refused by the backend, whatever the UI sends.
- **Verified before trashing.** Each item is checked against the scan first (same type, size and modification time); if it changed, it is left alone and you are asked to rescan.
- **Strict app sandbox.** Restrictive Content Security Policy, minimal Tauri permissions, `unsafe` Rust forbidden across the workspace.

## Performance targets

| Metric | Target |
| :--- | :--- |
| Directory scan (warm cache, NVMe) | ≥ 100,000 files/s on Linux, ≥ 60,000 files/s on Windows and macOS |
| NTFS MFT scan (`v0.3.0`) | 1 million files in < 3 s |
| Tree memory | ≤ 80 MB per 1,000,000 nodes |
| Folder view response | < 16 ms (one frame at 60 FPS) |
| Installer size | < 12 MB |

## Contributing

The project follows GitHub Flow: one roadmap step = one branch = one pull request, squash-merged into a protected `main`, with [Conventional Commits](https://www.conventionalcommits.org/) titles.
Every pull request must pass `cargo fmt`, `clippy -D warnings`, `cargo test` and `npm run build` on Windows, Linux and macOS.
For larger changes, please open an issue first.

## License

[MIT](LICENSE) © 2026 DiskSieve
