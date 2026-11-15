# Changelog

All notable changes to DiskSieve are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

The section of a version is also its GitHub release text and the update notes shown in the
app, so write it for users. Add changes under **Unreleased**; when releasing, rename that
heading to `## [x.y.z] - YYYY-MM-DD` and start a new empty **Unreleased** section.

## [Unreleased]

## [0.1.0] - 2026-11-15

The first public release: a fast, accurate and safe disk space analyzer.

### Added
- Multi-threaded scan of a whole drive or a single folder, with live progress and Cancel. Both size on disk and logical size are counted; the scan stays on one drive and never follows links.
- Folder list and treemap side by side: drill down, go up, breadcrumb, colors by file category, file age or dev junk.
- Space reconciliation: the "Unscanned / hidden" tile explains space the scan could not see (system files, restore points, folders without access) and lists skipped paths.
- Safe Move to Trash with Undo (Windows, Linux). DiskSieve refuses to touch system folders, your home folder, cloud-only files and links, and checks that a file has not changed since the scan.
- Dev Junk tab: build artifacts (`node_modules`, Rust `target`, Python venvs, Gradle, .NET `bin/obj` and 20 more) found only next to their project files, grouped by project, with "untouched for 90+ days" filter and bulk cleanup.
- `Ctrl+K` search by name across the whole scan, showing the largest matches first.
- Inspector with exact sizes, "Show in File Explorer" and "Copy Path".
- Automatic updates: DiskSieve looks for a new version on startup (can be turned off) and updates with one click. Updates are signature-checked before installing.
- Installers for Windows, macOS (Apple Silicon and Intel) and Linux, a portable Windows `.exe`, and `SHA256SUMS.txt` for every release.

[Unreleased]: https://github.com/DiskSieve/disksieve/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/DiskSieve/disksieve/releases/tag/v0.1.0
