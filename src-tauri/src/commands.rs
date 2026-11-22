//! `#[tauri::command]` handlers. Keep them thin: validate input, call `disksieve-core`,
//! map the result to a DTO from [`crate::dto`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use disksieve_core::{
    build_viewport, detect_junk, is_volume_root, junk_projects, load_previous,
    move_to_system_trash, restore_from_system_trash, review_sieve as review_basket, snapshot_path,
    Advisor, CoreError, Guard, Platform, ScanOptions, ScanTree, Snapshot, SpaceBalance,
    ViewportOptions, DEFAULT_SEARCH_LIMIT, RESTORE_SUPPORTED, ROOT,
};
use sysinfo::{DiskKind, Disks};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::dto::{
    AdviceDto, AppInfoDto, DevJunkDto, DriveDto, ScanProgressDto, ScanSummaryDto, SearchResultDto,
    SieveDto, SizeMetricDto, TrashReportDto, ViewportDto,
};
use crate::state::{AppState, Baseline};

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
            Ok(mut tree) => {
                // Dev Junk works on the tree alone (no disk access), so it is part of the scan.
                detect_junk(&mut tree);
                let scanned = tree.get(ROOT).map_or(0, |n| n.allocated_size);
                let mut summary = ScanSummaryDto::from_tree(&tree, volume_balance(&root, scanned));
                let (previous, warning) = rotate_snapshot(&app, &tree);
                summary.previous_snapshot_at = previous.as_ref().map(Snapshot::taken_at);
                summary.snapshot_warning = warning;
                let baseline = previous.map(|snapshot| Baseline::new(&tree, snapshot));
                Ok(state
                    .finish_scan(handle.generation, Some(tree), baseline)
                    .then_some(summary))
            }
            Err(CoreError::Cancelled) => {
                state.finish_scan(handle.generation, None, None);
                Ok(None)
            }
            Err(e) => {
                state.finish_scan(handle.generation, None, None);
                Err(e.to_string())
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Folder (inside the app data directory) with the latest snapshot of every scanned root.
const SNAPSHOT_DIR: &str = "snapshots";

/// Reads the previous snapshot of the scanned root (the Delta Mode baseline), then replaces
/// it with a snapshot of `tree`. Snapshot problems never fail the scan; they come back as a
/// warning for the UI.
fn rotate_snapshot(app: &AppHandle, tree: &ScanTree) -> (Option<Snapshot>, Option<String>) {
    let dir = match app.path().app_data_dir() {
        Ok(dir) => dir.join(SNAPSHOT_DIR),
        Err(e) => return (None, Some(format!("Scan snapshots are unavailable: {e}."))),
    };
    let root = tree.root_path();
    let mut warnings = Vec::new();
    let previous = load_previous(&dir, root).unwrap_or_else(|e| {
        warnings.push(format!("The previous scan snapshot was ignored: {e}."));
        None
    });
    if let Err(e) = Snapshot::capture(tree).save(&snapshot_path(&dir, root)) {
        warnings.push(format!("The scan snapshot could not be saved: {e}."));
    }
    let warning = (!warnings.is_empty()).then(|| warnings.join(" "));
    (previous, warning)
}

/// Cancels the scan in progress. Returns `false` when no scan was running.
#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>) -> bool {
    state.cancel_scan()
}

/// Projection of directory `node_id` (the scan root when `None`) from scan `generation`:
/// up to 200 largest children plus one "Other" node. Fails for a stale generation, so node
/// ids from an older scan can never address nodes of a newer tree.
///
/// Every node carries its change since the previous scan of the same root (when there is
/// one). With `delta`, only changed children are listed — the biggest changes, sorted by
/// growth — together with the items removed from this folder.
#[tauri::command]
pub async fn get_viewport(
    state: State<'_, AppState>,
    generation: u64,
    node_id: Option<u32>,
    metric: SizeMetricDto,
    delta: Option<bool>,
) -> Result<ViewportDto, String> {
    // `async` keeps the projection (up to ~200 ms for a directory with 1M entries) off the
    // main thread.
    let guard = state.tree();
    let tree = current(guard.as_ref(), generation)?;
    let baseline = state.baseline();
    let overlay = baseline.as_ref().map(|b| &b.delta);
    let delta_mode = delta.unwrap_or(false);
    if delta_mode && overlay.is_none() {
        return Err("there is no previous scan of this location to compare with".into());
    }
    let opts = ViewportOptions {
        metric: metric.into(),
        delta: overlay.filter(|_| delta_mode).map(|d| d.as_slice()),
        ..ViewportOptions::default()
    };
    let view = build_viewport(tree, node_id.unwrap_or(ROOT), &opts).map_err(|e| e.to_string())?;
    Ok(ViewportDto::from_viewport(tree, &view, overlay, delta_mode))
}

/// `Ctrl+K`: the largest files and folders of scan `generation` whose name contains every
/// word of `query` (case-insensitive). Searches the in-memory tree only — no disk access.
#[tauri::command]
pub async fn search_nodes(
    state: State<'_, AppState>,
    generation: u64,
    query: String,
    metric: SizeMetricDto,
    limit: Option<u32>,
) -> Result<SearchResultDto, String> {
    let guard = state.tree();
    let tree = current(guard.as_ref(), generation)?;
    let metric = metric.into();
    let limit = limit.map_or(DEFAULT_SEARCH_LIMIT, |l| (l as usize).min(MAX_SEARCH_LIMIT));
    let result = disksieve_core::search_nodes(tree, &query, metric, limit);
    Ok(SearchResultDto::new(tree, &query, metric, &result))
}

/// Upper bound for `search_nodes`' `limit`, so one IPC answer stays small.
const MAX_SEARCH_LIMIT: usize = 200;

/// Dev Junk of scan `generation`: projects with rebuildable artifacts, largest first.
/// Artifacts are detected when the scan finishes and refreshed after trash / restore.
#[tauri::command]
pub async fn get_dev_junk(
    state: State<'_, AppState>,
    generation: u64,
) -> Result<DevJunkDto, String> {
    let guard = state.tree();
    let tree = current(guard.as_ref(), generation)?;
    Ok(DevJunkDto::new(tree, &junk_projects(tree)))
}

/// Safety Advisor verdict ("Can I delete this?") for node `node_id` of scan `generation`.
/// Uses the same guard as `trash_nodes`, so `system` means the backend refuses to delete it.
/// Tree and path logic only — no disk access.
#[tauri::command]
pub async fn get_advice(
    app: AppHandle,
    state: State<'_, AppState>,
    generation: u64,
    node_id: u32,
) -> Result<AdviceDto, String> {
    let guard = delete_guard(&app);
    let advisor = Advisor::new(Platform::CURRENT, app.path().home_dir().ok().as_deref());
    let slot = state.tree();
    let tree = current(slot.as_ref(), generation)?;
    let advice = advisor
        .advise(tree, node_id, &guard)
        .map_err(|e| e.to_string())?;
    Ok(AdviceDto::new(generation, node_id, &advice))
}

/// Reviews the Sieve Basket — nodes collected for one "Move to Trash" — of scan `generation`:
/// Safety Advisor verdicts, items already inside another collected folder, items the guard
/// refuses, and the total that emptying the basket frees.
#[tauri::command]
pub async fn review_sieve(
    app: AppHandle,
    generation: u64,
    node_ids: Vec<u32>,
) -> Result<SieveDto, String> {
    let guard = delete_guard(&app);
    let advisor = Advisor::new(Platform::CURRENT, app.path().home_dir().ok().as_deref());
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = state.tree();
        let tree = current(slot.as_ref(), generation)?;
        let review = review_basket(tree, &node_ids, &advisor, &guard);
        Ok(SieveDto::new(tree, generation, &review))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Moves nodes of scan `generation` to the system Trash. Every node passes the backend guard
/// and an on-disk check (unchanged since the scan) first; the tree is updated after each
/// success, without a rescan. Returns one result per node — a failure never stops the rest.
#[tauri::command]
pub async fn trash_nodes(
    app: AppHandle,
    generation: u64,
    node_ids: Vec<u32>,
) -> Result<TrashReportDto, String> {
    let guard = delete_guard(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut slot = state.tree_mut();
        let tree = current_mut(slot.as_mut(), generation)?;
        let outcomes = disksieve_core::trash_nodes(tree, &node_ids, &guard, move_to_system_trash);
        // Folders above a removed artifact may no longer contain any.
        detect_junk(tree);
        if let Some(baseline) = state.baseline_mut().as_mut() {
            baseline.refresh(tree);
        }
        Ok(TrashReportDto::new(
            generation,
            &outcomes,
            RESTORE_SUPPORTED,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Restores nodes moved to Trash by `trash_nodes` ("Undo"; Windows and Linux only).
#[tauri::command]
pub async fn restore_nodes(
    app: AppHandle,
    generation: u64,
    node_ids: Vec<u32>,
) -> Result<TrashReportDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut slot = state.tree_mut();
        let tree = current_mut(slot.as_mut(), generation)?;
        let outcomes = disksieve_core::restore_nodes(tree, &node_ids, restore_from_system_trash);
        detect_junk(tree);
        if let Some(baseline) = state.baseline_mut().as_mut() {
            baseline.refresh(tree);
        }
        Ok(TrashReportDto::new(
            generation,
            &outcomes,
            RESTORE_SUPPORTED,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Opens the system file manager with the node selected.
#[tauri::command]
pub async fn reveal_in_explorer(
    app: AppHandle,
    state: State<'_, AppState>,
    generation: u64,
    node_id: u32,
) -> Result<(), String> {
    let path = {
        let guard = state.tree();
        let tree = current(guard.as_ref(), generation)?;
        let node = tree.node(node_id).map_err(|e| e.to_string())?;
        if node.is_trashed() {
            return Err("this item was moved to Trash".into());
        }
        tree.resolve_path(node_id).map_err(|e| e.to_string())?
    };
    tauri::async_runtime::spawn_blocking(move || app.opener().reveal_item_in_dir(path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Opens the Trash in Finder (macOS, where items cannot be restored programmatically).
#[tauri::command]
pub fn open_trash(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let trash = app
            .path()
            .home_dir()
            .map_err(|e| e.to_string())?
            .join(".Trash");
        app.opener()
            .open_path(trash.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("opening the Trash is only needed on macOS".into())
    }
}

/// The tree of scan `generation`; fails when nothing was scanned or the generation is stale,
/// so node ids from an older scan can never address nodes of a newer tree.
fn current(tree: Option<&ScanTree>, generation: u64) -> Result<&ScanTree, String> {
    let tree = tree.ok_or("no scan results yet")?;
    ensure_generation(tree, generation)?;
    Ok(tree)
}

fn current_mut(tree: Option<&mut ScanTree>, generation: u64) -> Result<&mut ScanTree, String> {
    let tree = tree.ok_or("no scan results yet")?;
    ensure_generation(tree, generation)?;
    Ok(tree)
}

fn ensure_generation(tree: &ScanTree, generation: u64) -> Result<(), String> {
    if tree.generation() == generation {
        Ok(())
    } else {
        Err(format!(
            "stale scan generation {generation} (current: {})",
            tree.generation()
        ))
    }
}

/// Delete guard protecting the home folder and DiskSieve's own install and data folders.
fn delete_guard(app: &AppHandle) -> Guard {
    let paths = app.path();
    let app_dirs = [
        paths.app_data_dir(),
        paths.app_local_data_dir(),
        paths.app_config_dir(),
        paths.app_cache_dir(),
        paths.app_log_dir(),
        paths.resource_dir(),
    ]
    .into_iter()
    .filter_map(Result::ok)
    .chain(
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
    );
    Guard::new(paths.home_dir().ok(), app_dirs)
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
