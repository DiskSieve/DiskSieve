//! `#[tauri::command]` handlers. Keep them thin: validate input, call `disksieve-core`,
//! map the result to a DTO from [`crate::dto`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use disksieve_core::{
    build_viewport, is_volume_root, CoreError, ScanOptions, SpaceBalance, ViewportOptions, ROOT,
};
use sysinfo::{DiskKind, Disks};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::dto::{
    AppInfoDto, DriveDto, ScanProgressDto, ScanSummaryDto, SizeMetricDto, ViewportDto,
};
use crate::state::AppState;

/// Returns static information about the running application.
#[tauri::command]
pub fn app_info() -> AppInfoDto {
    AppInfoDto::current()
}

/// Lists mounted volumes that can be scanned, sorted by mount point.
#[tauri::command]
pub async fn list_drives() -> Result<Vec<DriveDto>, String> {
    tauri::async_runtime::spawn_blocking(list_volumes)
        .await
        .map_err(|e| e.to_string())
}

/// Opens the native folder picker. Returns `None` when the user cancels.
#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a folder to scan")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned()))
}

/// Scans `path`, streaming progress through `on_progress`. Starting a scan cancels the one in
/// progress. Returns the summary, or `None` if the scan was cancelled or superseded by a newer
/// one (the previous tree is then kept).
#[tauri::command]
pub async fn start_scan(
    app: AppHandle,
    path: String,
    on_progress: Channel<ScanProgressDto>,
) -> Result<Option<ScanSummaryDto>, String> {
    let root = PathBuf::from(path);
    let handle = app.state::<AppState>().begin_scan();

    tauri::async_runtime::spawn_blocking(move || {
        let opts = ScanOptions {
            generation: handle.generation,
            ..ScanOptions::default()
        };
        let threads = opts.effective_threads();
        let started = Instant::now();
        // The scanner already throttles callbacks to `progress_interval` (100 ms).
        let outcome = disksieve_core::scan(&root, &opts, &handle.cancel, |p| {
            if on_progress
                .send(ScanProgressDto::new(
                    handle.generation,
                    p,
                    started.elapsed(),
                    threads,
                ))
                .is_err()
            {
                // The window is gone; nobody is waiting for this scan.
                handle.cancel.cancel();
            }
        });

        let state = app.state::<AppState>();
        match outcome {
            Ok(tree) => {
                let scanned = tree.get(ROOT).map_or(0, |n| n.allocated_size);
                let summary = ScanSummaryDto::from_tree(&tree, volume_balance(&root, scanned));
                Ok(state
                    .finish_scan(handle.generation, Some(tree))
                    .then_some(summary))
            }
            Err(CoreError::Cancelled) => {
                state.finish_scan(handle.generation, None);
                Ok(None)
            }
            Err(e) => {
                state.finish_scan(handle.generation, None);
                Err(e.to_string())
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Cancels the scan in progress. Returns `false` when no scan was running.
#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>) -> bool {
    state.cancel_scan()
}

/// Projection of directory `node_id` (the scan root when `None`) from scan `generation`:
/// up to 200 largest children plus one "Other" node. Fails for a stale generation, so node
/// ids from an older scan can never address nodes of a newer tree.
#[tauri::command]
pub async fn get_viewport(
    state: State<'_, AppState>,
    generation: u64,
    node_id: Option<u32>,
    metric: SizeMetricDto,
) -> Result<ViewportDto, String> {
    // `async` keeps the projection (up to ~200 ms for a directory with 1M entries) off the
    // main thread.
    let guard = state.tree();
    let tree = guard.as_ref().ok_or("no scan results yet")?;
    if tree.generation() != generation {
        return Err(format!(
            "stale scan generation {generation} (current: {})",
            tree.generation()
        ));
    }
    let opts = ViewportOptions {
        metric: metric.into(),
        ..ViewportOptions::default()
    };
    let view = build_viewport(tree, node_id.unwrap_or(ROOT), &opts).map_err(|e| e.to_string())?;
    Ok(ViewportDto::from_viewport(tree, &view))
}

/// Space balance when `root` is the mount point of a volume; `None` for a folder scan.
/// Usage is read after the scan, so it matches the moment the tree was completed.
fn volume_balance(root: &Path, scanned: u64) -> Option<SpaceBalance> {
    Disks::new_with_refreshed_list()
        .list()
        .iter()
        .find(|d| is_volume_root(root, d.mount_point()))
        .map(|d| SpaceBalance::new(d.total_space(), d.available_space(), scanned))
}

/// Pseudo and image file systems that are never worth scanning.
const IGNORED_FILE_SYSTEMS: &[&str] = &["squashfs", "overlay", "tmpfs", "devtmpfs", "ramfs"];

fn list_volumes() -> Vec<DriveDto> {
    let disks = Disks::new_with_refreshed_list();
    let mut seen = HashSet::new();
    let mut drives: Vec<DriveDto> = disks
        .list()
        .iter()
        .filter(|d| d.total_space() > 0)
        .filter(|d| {
            let fs = d.file_system().to_string_lossy();
            !IGNORED_FILE_SYSTEMS.contains(&fs.as_ref())
        })
        .filter(|d| seen.insert(d.mount_point().to_path_buf()))
        .map(|d| DriveDto {
            mount_point: d.mount_point().to_string_lossy().into_owned(),
            name: d.name().to_string_lossy().into_owned(),
            file_system: d.file_system().to_string_lossy().into_owned(),
            kind: match d.kind() {
                DiskKind::SSD => "ssd",
                DiskKind::HDD => "hdd",
                DiskKind::Unknown(_) => "unknown",
            }
            .into(),
            total_bytes: d.total_space(),
            available_bytes: d.available_space(),
            is_removable: d.is_removable(),
        })
        .collect();
    drives.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    drives
}
