//! IPC data transfer objects — the only types that cross the Rust ↔ WebView boundary.
//!
//! Every DTO derives `TS` and is exported to `src/types/generated/` (see `.cargo/config.toml`).
//! Regenerate the TypeScript bindings with `npm run gen:types`; never edit them by hand.
//! Rules: `#[serde(rename_all = "camelCase")]`, `#[ts(export)]`, and `#[ts(type = "number")]`
//! on 64-bit integers (values stay below 2^53).

use disksieve_core::{
    contains_junk, Advice, Change, ChangeItem, ChangeKind, ChangeList, Delta, DuplicatePhase,
    DuplicateProgress, DuplicateReport, JunkKind, JunkProject, Node, NodeFlags, NodeId,
    OtherBucket, SafetyLevel, ScanProgress, ScanTree, SearchResult, SieveReview, SieveStatus,
    SizeMetric, SpaceBalance, TrashError, TrashOutcome, Viewport, ROOT,
};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Static information about the running application, shown in the header and About dialog.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfoDto {
    /// Product name, e.g. `DiskSieve`.
    pub name: String,
    /// Desktop app version (from the workspace manifest).
    pub version: String,
    /// `disksieve-core` engine version.
    pub core_version: String,
    /// Operating system family: `windows`, `linux` or `macos`.
    pub os: String,
    /// CPU architecture, e.g. `x86_64` or `aarch64`.
    pub arch: String,
}

impl AppInfoDto {
    pub fn current() -> Self {
        Self {
            name: "DiskSieve".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            core_version: disksieve_core::VERSION.into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
        }
    }
}

/// A mounted volume that can be scanned.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DriveDto {
    /// Scan root, e.g. `C:\` or `/`.
    pub mount_point: String,
    /// Volume label (may be empty).
    pub name: String,
    /// File system, e.g. `NTFS`, `apfs`, `ext4`.
    pub file_system: String,
    /// `ssd`, `hdd` or `unknown`.
    pub kind: String,
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number")]
    pub available_bytes: u64,
    pub is_removable: bool,
}

/// Progress of a running scan, streamed through a `Channel` at most ~10 times per second.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScanProgressDto {
    #[ts(type = "number")]
    pub generation: u64,
    #[ts(type = "number")]
    pub files: u64,
    #[ts(type = "number")]
    pub dirs: u64,
    /// Size on disk found so far.
    #[ts(type = "number")]
    pub allocated_bytes: u64,
    #[ts(type = "number")]
    pub skipped_entries: u64,
    /// Directory processed most recently (display only).
    pub current_dir: String,
    /// Time since the scan started.
    #[ts(type = "number")]
    pub elapsed_ms: u64,
    /// Worker threads reading directories.
    pub threads: u32,
}

impl ScanProgressDto {
    pub fn new(generation: u64, p: &ScanProgress, elapsed: Duration, threads: usize) -> Self {
        Self {
            generation,
            files: p.files,
            dirs: p.dirs,
            allocated_bytes: p.allocated_bytes,
            skipped_entries: p.skipped_entries,
            current_dir: p.current_dir.to_string_lossy().into_owned(),
            elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            threads: u32::try_from(threads).unwrap_or(u32::MAX),
        }
    }
}

/// Result of a completed scan. The tree itself stays in Rust.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScanSummaryDto {
    #[ts(type = "number")]
    pub generation: u64,
    pub root_path: String,
    pub file_count: u32,
    pub dir_count: u32,
    #[ts(type = "number")]
    pub logical_bytes: u64,
    #[ts(type = "number")]
    pub allocated_bytes: u64,
    #[ts(type = "number")]
    pub skipped_entries: u64,
    /// Up to 50 example paths of skipped entries.
    pub skipped_samples: Vec<String>,
    #[ts(type = "number")]
    pub duration_ms: u64,
    /// Volume usage vs. scan result; `null` unless a whole volume was scanned.
    pub volume: Option<SpaceBalanceDto>,
    /// When the previous snapshot of this root was taken (seconds since the UNIX epoch);
    /// `null` on the first scan of this root.
    pub previous_snapshot_at: Option<u32>,
    /// Why the previous snapshot was ignored or the new one was not saved; `null` when fine.
    pub snapshot_warning: Option<String>,
}

/// Space reconciliation of a whole-volume scan (all values in bytes).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpaceBalanceDto {
    #[ts(type = "number")]
    pub total_bytes: u64,
    /// Used space reported by the OS.
    #[ts(type = "number")]
    pub used_bytes: u64,
    #[ts(type = "number")]
    pub free_bytes: u64,
    /// Size on disk found by the scan.
    #[ts(type = "number")]
    pub scanned_bytes: u64,
    /// Used space the scan could not see ("Unscanned / hidden").
    #[ts(type = "number")]
    pub unaccounted_bytes: u64,
}

impl From<SpaceBalance> for SpaceBalanceDto {
    fn from(b: SpaceBalance) -> Self {
        Self {
            total_bytes: b.total,
            used_bytes: b.used,
            free_bytes: b.free,
            scanned_bytes: b.scanned,
            unaccounted_bytes: b.unaccounted,
        }
    }
}

impl ScanSummaryDto {
    /// `volume` is the balance of a whole-volume scan (`None` for a folder scan).
    pub fn from_tree(tree: &ScanTree, volume: Option<SpaceBalance>) -> Self {
        let (file_count, logical_bytes, allocated_bytes) = tree.get(ROOT).map_or((0, 0, 0), |n| {
            (n.file_count, n.logical_size, n.allocated_size)
        });
        let dir_count = (0..tree.len())
            .filter_map(|i| tree.get(i as u32))
            .filter(|n| n.flags.contains(NodeFlags::DIR))
            .count();
        Self {
            generation: tree.generation(),
            root_path: tree.root_path().to_string_lossy().into_owned(),
            file_count,
            dir_count: u32::try_from(dir_count).unwrap_or(u32::MAX),
            logical_bytes,
            allocated_bytes,
            skipped_entries: tree.stats().skipped_entries,
            skipped_samples: tree
                .stats()
                .skipped_samples
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            duration_ms: u64::try_from(tree.stats().duration.as_millis()).unwrap_or(u64::MAX),
            volume: volume.map(Into::into),
            previous_snapshot_at: None,
            snapshot_warning: None,
        }
    }
}

/// Size used for treemap areas and ordering. `delta` arrives with snapshots in v0.2.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SizeMetricDto {
    Allocated,
    Logical,
}

impl From<SizeMetricDto> for SizeMetric {
    fn from(m: SizeMetricDto) -> Self {
        match m {
            SizeMetricDto::Allocated => Self::Allocated,
            SizeMetricDto::Logical => Self::Logical,
        }
    }
}

impl From<SizeMetric> for SizeMetricDto {
    fn from(m: SizeMetric) -> Self {
        match m {
            SizeMetric::Allocated => Self::Allocated,
            SizeMetric::Logical => Self::Logical,
        }
    }
}

/// One step of the path from the scan root to the current directory.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BreadcrumbDto {
    pub id: u32,
    pub name: String,
}

/// One tile of the treemap / row of the list.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TreemapNodeDto {
    /// `null` only for the aggregated "Other" node.
    pub id: Option<u32>,
    pub name: String,
    /// Display path (may contain replacement characters for non-Unicode names).
    pub path: String,
    /// Value under the requested metric (0 for hard links counted elsewhere).
    #[ts(type = "number")]
    pub value: u64,
    #[ts(type = "number")]
    pub logical_size: u64,
    #[ts(type = "number")]
    pub allocated_size: u64,
    /// Change in bytes on disk since the previous scan of this location; `null` when there
    /// is none (and for "Other" outside Delta Mode).
    #[ts(type = "number | null")]
    pub delta_bytes: Option<i64>,
    pub is_dir: bool,
    pub file_count: u32,
    /// Newest modification time in the subtree, seconds since the UNIX epoch.
    pub newest_mtime: u32,
    /// Lower-case extension without the dot; `null` for folders and files without one.
    pub extension: Option<String>,
    /// Dev-junk kind of this folder, e.g. `node_modules` or `cargo_target`.
    pub junk_kind: Option<String>,
    /// e.g. `["hardlink", "cloudOnline", "accessDenied"]`; `containsJunk` for a folder with
    /// dev artifacts somewhere below it.
    pub flags: Vec<String>,
    /// Number of merged children (only for "Other").
    pub other_count: Option<u32>,
}

impl TreemapNodeDto {
    fn from_node(tree: &ScanTree, id: NodeId, node: &Node, value: u64) -> Self {
        let ext = tree.extension(id);
        let junk_kind = JunkKind::of(tree, id).map(|k| k.as_str().to_owned());
        let mut flags = flag_names(node.flags);
        if junk_kind.is_none() && contains_junk(tree, id) {
            flags.push(CONTAINS_JUNK_FLAG.into());
        }
        Self {
            id: Some(id),
            name: tree.name(id).to_owned(),
            path: tree.display_path(id),
            value,
            logical_size: node.logical_size,
            allocated_size: node.allocated_size,
            delta_bytes: None,
            is_dir: node.is_dir(),
            file_count: node.file_count,
            newest_mtime: node.newest_mtime,
            extension: (!ext.is_empty()).then(|| ext.to_owned()),
            junk_kind,
            flags,
            other_count: None,
        }
    }

    fn other(current_path: &str, bucket: &OtherBucket) -> Self {
        Self {
            id: None,
            name: "Other".into(),
            path: current_path.to_owned(),
            value: bucket.value,
            logical_size: bucket.logical_size,
            allocated_size: bucket.allocated_size,
            delta_bytes: None,
            is_dir: false,
            file_count: bucket.file_count,
            newest_mtime: bucket.newest_mtime,
            extension: None,
            junk_kind: None,
            flags: Vec::new(),
            other_count: Some(bucket.count),
        }
    }
}

/// Projection of one directory: at most 200 largest children plus one "Other" node.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ViewportDto {
    /// Scan generation; the UI drops responses from an older scan.
    #[ts(type = "number")]
    pub generation: u64,
    pub current_id: u32,
    pub current_path: String,
    /// From the scan root to the current directory (inclusive).
    pub breadcrumbs: Vec<BreadcrumbDto>,
    pub metric: SizeMetricDto,
    #[ts(type = "number")]
    pub total_value: u64,
    /// Change of the current directory since the previous scan; `null` when there is none.
    #[ts(type = "number | null")]
    pub delta_bytes: Option<i64>,
    /// `true` when only changed children are listed, biggest growth first.
    pub delta_mode: bool,
    pub nodes: Vec<TreemapNodeDto>,
    /// Delta Mode: items of the previous scan that are gone from this directory, largest
    /// first (at most [`MAX_REMOVED_ITEMS`]).
    pub removed: Vec<RemovedItemDto>,
    /// All removed items of this directory, including those not listed.
    pub removed_count: u32,
}

/// Most removed items sent with one viewport.
pub const MAX_REMOVED_ITEMS: usize = 200;

/// Kind of one line of the Changes view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChangeKindDto {
    Added,
    Removed,
    Grew,
    Shrank,
    SmallFiles,
}

impl From<ChangeKindDto> for ChangeKind {
    fn from(k: ChangeKindDto) -> Self {
        match k {
            ChangeKindDto::Added => Self::Added,
            ChangeKindDto::Removed => Self::Removed,
            ChangeKindDto::Grew => Self::Grew,
            ChangeKindDto::Shrank => Self::Shrank,
            ChangeKindDto::SmallFiles => Self::SmallFiles,
        }
    }
}

impl From<ChangeKind> for ChangeKindDto {
    fn from(k: ChangeKind) -> Self {
        match k {
            ChangeKind::Added => Self::Added,
            ChangeKind::Removed => Self::Removed,
            ChangeKind::Grew => Self::Grew,
            ChangeKind::Shrank => Self::Shrank,
            ChangeKind::SmallFiles => Self::SmallFiles,
        }
    }
}

/// One line of the Changes view.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangeDto {
    pub kind: ChangeKindDto,
    /// Item of the current scan (added, grew, shrank); `null` for removed items and small files.
    pub node_id: Option<u32>,
    /// Folder to open in the Explorer to see the change.
    pub folder_id: u32,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// Change in bytes on disk (negative for shrank and removed).
    #[ts(type = "number")]
    pub bytes: i64,
    /// Size on disk in the previous scan; `null` when unknown (added items, small files).
    #[ts(type = "number | null")]
    pub before_size: Option<u64>,
    /// Size on disk now; `null` for removed items and small files.
    #[ts(type = "number | null")]
    pub after_size: Option<u64>,
}

/// Number and sum of the changes of one kind.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangeTotalDto {
    pub kind: ChangeKindDto,
    pub count: u32,
    #[ts(type = "number")]
    pub bytes: i64,
}

/// Changes since the previous scan, like `git diff --stat`. The entries of all kinds add up
/// to `net`, the change of the scan root.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangesDto {
    #[ts(type = "number")]
    pub generation: u64,
    /// When the compared scan started, seconds since the UNIX epoch.
    pub previous_scan_at: u32,
    /// Filter the list was built with (`null` = all kinds).
    pub kind: Option<ChangeKindDto>,
    /// Largest changes first (at most the requested limit).
    pub changes: Vec<ChangeDto>,
    /// Changes matching the filter, including those not listed.
    pub matching: u32,
    /// Totals of every kind, independent of the filter.
    pub totals: Vec<ChangeTotalDto>,
    #[ts(type = "number")]
    pub grown: u64,
    #[ts(type = "number")]
    pub shrunk: u64,
    #[ts(type = "number")]
    pub net: i64,
}

/// Label of the "small files" line of a folder.
pub const SMALL_FILES_LABEL: &str = "Small files (< 1 MiB)";

impl ChangesDto {
    pub fn new(
        tree: &ScanTree,
        delta: &Delta,
        kind: Option<ChangeKindDto>,
        list: &ChangeList,
    ) -> Self {
        let changes = list
            .changes
            .iter()
            .filter_map(|c| change_dto(tree, delta, c))
            .collect();
        Self {
            generation: tree.generation(),
            previous_scan_at: delta.taken_at(),
            kind,
            changes,
            matching: list.matching,
            totals: ChangeKind::ALL
                .iter()
                .map(|&k| {
                    let t = list.total(k);
                    ChangeTotalDto {
                        kind: k.into(),
                        count: t.count,
                        bytes: t.bytes,
                    }
                })
                .collect(),
            grown: list.grown,
            shrunk: list.shrunk,
            net: list.net(),
        }
    }
}

fn change_dto(tree: &ScanTree, delta: &Delta, c: &Change) -> Option<ChangeDto> {
    let kind = c.kind.into();
    Some(match c.item {
        ChangeItem::Node(id) => {
            let node = tree.get(id)?;
            ChangeDto {
                kind,
                node_id: Some(id),
                folder_id: if id == ROOT { ROOT } else { node.parent },
                name: tree.name(id).to_owned(),
                path: tree.display_path(id),
                is_dir: node.is_dir(),
                bytes: c.bytes,
                before_size: delta.before(id),
                after_size: Some(node.contribution().1),
            }
        }
        ChangeItem::Removed(i) => {
            let r = delta.removed().get(i)?;
            ChangeDto {
                kind,
                node_id: None,
                folder_id: r.parent,
                name: r.name.clone(),
                path: std::path::Path::new(&tree.display_path(r.parent))
                    .join(&r.name)
                    .to_string_lossy()
                    .into_owned(),
                is_dir: r.is_dir,
                bytes: c.bytes,
                before_size: Some(r.allocated),
                after_size: None,
            }
        }
        ChangeItem::SmallFiles(folder) => ChangeDto {
            kind,
            node_id: None,
            folder_id: folder,
            name: SMALL_FILES_LABEL.to_owned(),
            path: tree.display_path(folder),
            is_dir: false,
            bytes: c.bytes,
            before_size: None,
            after_size: None,
        },
    })
}

/// Pass of the duplicate search that is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DuplicatePhaseDto {
    /// Hashing the first and last 16 KiB of every file of a shared size.
    Quick,
    /// Hashing the whole content of the files that passed the quick check.
    Full,
}

/// Progress of a duplicate search, streamed through a `Channel` at most ~10 times per second.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateProgressDto {
    #[ts(type = "number")]
    pub generation: u64,
    pub phase: DuplicatePhaseDto,
    pub files_done: u32,
    pub files_total: u32,
    #[ts(type = "number")]
    pub bytes_done: u64,
    #[ts(type = "number")]
    pub bytes_total: u64,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
}

impl DuplicateProgressDto {
    pub fn new(generation: u64, p: &DuplicateProgress, elapsed: Duration) -> Self {
        Self {
            generation,
            phase: match p.phase {
                DuplicatePhase::Quick => DuplicatePhaseDto::Quick,
                DuplicatePhase::Full => DuplicatePhaseDto::Full,
            },
            files_done: p.files_done,
            files_total: p.files_total,
            bytes_done: p.bytes_done,
            bytes_total: p.bytes_total,
            elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        }
    }
}

/// One copy of a duplicated file.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateCopyDto {
    pub node_id: u32,
    /// Folder holding the copy (to open it in the Explorer).
    pub folder_id: u32,
    pub path: String,
    /// Modification time, seconds since the UNIX epoch.
    pub mtime: u32,
}

/// Files with identical content; copies oldest first (the one suggested to keep).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateGroupDto {
    /// `blake3` of the content, 64 hex digits.
    pub hash: String,
    /// Name of the oldest copy.
    pub name: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
    /// Freed by keeping one copy: `size × (copies − 1)`.
    #[ts(type = "number")]
    pub wasted_bytes: u64,
    pub copies: Vec<DuplicateCopyDto>,
}

/// Result of the duplicate search of scan `generation`. Copies trashed since the search are
/// left out, and so are groups with fewer than two copies left.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicatesDto {
    #[ts(type = "number")]
    pub generation: u64,
    /// Most wasted space first, at most [`MAX_DUPLICATE_GROUPS`].
    pub groups: Vec<DuplicateGroupDto>,
    /// All groups, including those not listed.
    pub group_count: u32,
    /// Copies in all groups.
    pub copy_count: u32,
    /// Freed by keeping one copy of every group.
    #[ts(type = "number")]
    pub wasted_bytes: u64,
    /// Files compared (same size as another file).
    pub files_checked: u32,
    /// Bytes read from disk by the search.
    #[ts(type = "number")]
    pub bytes_read: u64,
    /// Files that could not be read or changed since the scan.
    pub skipped: u32,
    #[ts(type = "number")]
    pub duration_ms: u64,
}

/// Groups sent to the UI at most, so one IPC answer stays small.
pub const MAX_DUPLICATE_GROUPS: usize = 1_000;

impl DuplicatesDto {
    pub fn new(tree: &ScanTree, report: &DuplicateReport, duration: Duration) -> Self {
        let alive = |id: NodeId| {
            tree.get(id).is_some_and(|n| !n.is_trashed())
                && !tree
                    .ancestors(id)
                    .any(|a| tree.get(a).is_some_and(Node::is_trashed))
        };
        let mut groups: Vec<DuplicateGroupDto> = report
            .groups
            .iter()
            .filter_map(|g| {
                let copies: Vec<DuplicateCopyDto> = g
                    .copies
                    .iter()
                    .filter(|c| alive(c.id))
                    .map(|c| DuplicateCopyDto {
                        node_id: c.id,
                        folder_id: tree.get(c.id).map_or(ROOT, |n| n.parent),
                        path: tree.display_path(c.id),
                        mtime: c.mtime,
                    })
                    .collect();
                (copies.len() >= 2).then(|| DuplicateGroupDto {
                    hash: g.hash_hex(),
                    name: tree.name(copies[0].node_id).to_owned(),
                    size_bytes: g.size,
                    wasted_bytes: g.size.saturating_mul(copies.len() as u64 - 1),
                    copies,
                })
            })
            .collect();
        groups.sort_by_key(|g| std::cmp::Reverse(g.wasted_bytes));
        let group_count = u32::try_from(groups.len()).unwrap_or(u32::MAX);
        let copy_count = groups.iter().map(|g| g.copies.len()).sum::<usize>();
        let wasted_bytes = groups.iter().map(|g| g.wasted_bytes).sum();
        groups.truncate(MAX_DUPLICATE_GROUPS);
        Self {
            generation: tree.generation(),
            groups,
            group_count,
            copy_count: u32::try_from(copy_count).unwrap_or(u32::MAX),
            wasted_bytes,
            files_checked: report.files_checked,
            bytes_read: report.bytes_read,
            skipped: report.skipped,
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        }
    }
}

/// A file or folder of the previous scan that is gone now (deleted, moved or trashed).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemovedItemDto {
    pub name: String,
    pub path: String,
    /// Size on disk in the previous scan.
    #[ts(type = "number")]
    pub allocated_size: u64,
    pub is_dir: bool,
}

impl ViewportDto {
    /// `delta` is the overlay of the current tree (when a previous scan exists); `delta_mode`
    /// says whether `view` was built with it.
    pub fn from_viewport(
        tree: &ScanTree,
        view: &Viewport,
        delta: Option<&Delta>,
        delta_mode: bool,
    ) -> Self {
        let current_path = tree.display_path(view.current);
        let mut nodes: Vec<TreemapNodeDto> = view
            .items
            .iter()
            .filter_map(|item| {
                let node = tree.get(item.id)?;
                let mut dto = TreemapNodeDto::from_node(tree, item.id, node, item.value);
                dto.delta_bytes = delta.and_then(|d| d.bytes(item.id));
                Some(dto)
            })
            .collect();
        if let Some(bucket) = &view.other {
            let mut other = TreemapNodeDto::other(&current_path, bucket);
            other.delta_bytes = delta_mode.then_some(bucket.delta_bytes);
            nodes.push(other);
        }
        let removed_here = match delta {
            Some(d) if delta_mode => d.removed_in(view.current),
            _ => &[],
        };
        let removed = removed_here
            .iter()
            .take(MAX_REMOVED_ITEMS)
            .map(|r| RemovedItemDto {
                path: std::path::Path::new(&current_path)
                    .join(&r.name)
                    .to_string_lossy()
                    .into_owned(),
                name: r.name.clone(),
                allocated_size: r.allocated,
                is_dir: r.is_dir,
            })
            .collect();
        Self {
            delta_bytes: delta.and_then(|d| d.bytes(view.current)),
            delta_mode,
            removed,
            removed_count: u32::try_from(removed_here.len()).unwrap_or(u32::MAX),
            generation: view.generation,
            current_id: view.current,
            breadcrumbs: view
                .breadcrumbs
                .iter()
                .map(|&id| BreadcrumbDto {
                    id,
                    name: tree.name(id).to_owned(),
                })
                .collect(),
            current_path,
            metric: view.metric.into(),
            total_value: view.total_value,
            nodes,
        }
    }
}

/// One `Ctrl+K` search hit: the node plus the folder it lives in (for navigation).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHitDto {
    pub node: TreemapNodeDto,
    /// Folder to open in the Explorer so the hit is visible.
    pub parent_id: u32,
}

/// Largest nodes whose name matches a query (searched in memory, never on disk).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchResultDto {
    #[ts(type = "number")]
    pub generation: u64,
    /// The query as received, so the UI can drop answers to an older keystroke.
    pub query: String,
    pub metric: SizeMetricDto,
    /// Total number of matches; `hits` holds at most the requested limit.
    pub total_matches: u32,
    pub hits: Vec<SearchHitDto>,
}

impl SearchResultDto {
    pub fn new(tree: &ScanTree, query: &str, metric: SizeMetric, result: &SearchResult) -> Self {
        Self {
            generation: tree.generation(),
            query: query.to_owned(),
            metric: metric.into(),
            total_matches: u32::try_from(result.total_matches).unwrap_or(u32::MAX),
            hits: result
                .hits
                .iter()
                .filter_map(|&id| {
                    let node = tree.get(id)?;
                    Some(SearchHitDto {
                        node: TreemapNodeDto::from_node(tree, id, node, metric.value(node)),
                        parent_id: node.parent,
                    })
                })
                .collect(),
        }
    }
}

/// Why one entry of a trash / restore request failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TrashErrorKindDto {
    /// Refused by the backend guard (protected location, link, cloud file, …).
    Protected,
    /// The entry changed on disk since the scan.
    Changed,
    /// The entry no longer exists.
    Missing,
    /// The Trash operation itself failed (permissions, file in use, …).
    Failed,
}

/// Result for one requested node.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrashItemDto {
    pub id: u32,
    /// Display path.
    pub path: String,
    /// `null` on success.
    pub error: Option<String>,
    pub error_kind: Option<TrashErrorKindDto>,
    /// Size on disk moved to (or restored from) Trash; 0 on failure.
    #[ts(type = "number")]
    pub allocated_bytes: u64,
    #[ts(type = "number")]
    pub logical_bytes: u64,
    pub file_count: u32,
}

impl From<&TrashOutcome> for TrashItemDto {
    fn from(o: &TrashOutcome) -> Self {
        let (freed, error, error_kind) = match &o.result {
            Ok(freed) => (*freed, None, None),
            Err(e) => {
                let kind = match e {
                    TrashError::InvalidNode(_) | TrashError::Refused(_) => {
                        TrashErrorKindDto::Protected
                    }
                    TrashError::Changed(_) => TrashErrorKindDto::Changed,
                    TrashError::Missing => TrashErrorKindDto::Missing,
                    TrashError::Io(_) => TrashErrorKindDto::Failed,
                };
                (Default::default(), Some(e.to_string()), Some(kind))
            }
        };
        Self {
            id: o.id,
            path: o.path.clone(),
            error,
            error_kind,
            allocated_bytes: freed.allocated,
            logical_bytes: freed.logical,
            file_count: freed.files,
        }
    }
}

/// Outcome of `trash_nodes` / `restore_nodes`: one item per requested node plus totals of
/// the successful ones.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrashReportDto {
    #[ts(type = "number")]
    pub generation: u64,
    pub items: Vec<TrashItemDto>,
    #[ts(type = "number")]
    pub allocated_bytes: u64,
    #[ts(type = "number")]
    pub logical_bytes: u64,
    pub file_count: u32,
    /// Whether moved items can be restored with `restore_nodes` (Windows, Linux).
    pub undo_available: bool,
}

impl TrashReportDto {
    pub fn new(generation: u64, outcomes: &[TrashOutcome], undo_available: bool) -> Self {
        let items: Vec<TrashItemDto> = outcomes.iter().map(TrashItemDto::from).collect();
        Self {
            generation,
            allocated_bytes: items.iter().map(|i| i.allocated_bytes).sum(),
            logical_bytes: items.iter().map(|i| i.logical_bytes).sum(),
            file_count: items.iter().map(|i| i.file_count).sum(),
            items,
            undo_available,
        }
    }
}

/// Flag of a folder with dev artifacts somewhere below it (treemap "junk" color mode).
const CONTAINS_JUNK_FLAG: &str = "containsJunk";

/// One rebuildable artifact of a project.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JunkArtifactDto {
    pub id: u32,
    /// Folder name, e.g. `node_modules`.
    pub name: String,
    /// Path relative to the project folder, e.g. `src/__pycache__`.
    pub rel_path: String,
    /// Full display path.
    pub path: String,
    /// Stable kind identifier, e.g. `node_modules`, `cargo_target`.
    pub kind: String,
    #[ts(type = "number")]
    pub allocated_bytes: u64,
    #[ts(type = "number")]
    pub logical_bytes: u64,
    pub file_count: u32,
}

/// A project with rebuildable artifacts.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JunkProjectDto {
    /// Node id of the project folder.
    pub id: u32,
    pub name: String,
    pub path: String,
    /// Newest file outside artifacts (seconds since the UNIX epoch, 0 if none) — when the
    /// project was last worked on.
    pub newest_mtime: u32,
    #[ts(type = "number")]
    pub allocated_bytes: u64,
    #[ts(type = "number")]
    pub logical_bytes: u64,
    pub file_count: u32,
    /// Largest first.
    pub artifacts: Vec<JunkArtifactDto>,
}

/// Result of `get_dev_junk`: projects with artifacts, largest first, plus totals.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DevJunkDto {
    #[ts(type = "number")]
    pub generation: u64,
    pub projects: Vec<JunkProjectDto>,
    #[ts(type = "number")]
    pub allocated_bytes: u64,
    #[ts(type = "number")]
    pub logical_bytes: u64,
    pub artifact_count: u32,
}

impl DevJunkDto {
    pub fn new(tree: &ScanTree, projects: &[JunkProject]) -> Self {
        let projects: Vec<JunkProjectDto> = projects
            .iter()
            .map(|p| JunkProjectDto {
                id: p.dir,
                name: tree.name(p.dir).to_owned(),
                path: tree.display_path(p.dir),
                newest_mtime: p.newest_mtime,
                allocated_bytes: p.allocated_size,
                logical_bytes: p.logical_size,
                file_count: p.file_count,
                artifacts: p
                    .artifacts
                    .iter()
                    .filter_map(|&id| artifact(tree, p.dir, id))
                    .collect(),
            })
            .collect();
        Self {
            generation: tree.generation(),
            allocated_bytes: projects.iter().map(|p| p.allocated_bytes).sum(),
            logical_bytes: projects.iter().map(|p| p.logical_bytes).sum(),
            artifact_count: projects.iter().map(|p| p.artifacts.len() as u32).sum(),
            projects,
        }
    }
}

fn artifact(tree: &ScanTree, project: NodeId, id: NodeId) -> Option<JunkArtifactDto> {
    let node = tree.get(id)?;
    let (logical, allocated, files) = node.contribution();
    // Names from the artifact up to (not including) the project folder.
    let mut parts = vec![tree.name(id)];
    parts.extend(
        tree.ancestors(id)
            .take_while(|&a| a != project)
            .map(|a| tree.name(a)),
    );
    parts.reverse();
    Some(JunkArtifactDto {
        id,
        name: tree.name(id).to_owned(),
        rel_path: parts.join("/"),
        path: tree.display_path(id),
        kind: JunkKind::of(tree, id)?.as_str().to_owned(),
        allocated_bytes: allocated,
        logical_bytes: logical,
        file_count: files,
    })
}

/// Safety Advisor level: `system` exactly when the backend refuses to move the item to Trash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SafetyLevelDto {
    Safe,
    Review,
    System,
}

impl From<SafetyLevel> for SafetyLevelDto {
    fn from(level: SafetyLevel) -> Self {
        match level {
            SafetyLevel::Safe => Self::Safe,
            SafetyLevel::Review => Self::Review,
            SafetyLevel::System => Self::System,
        }
    }
}

/// A safer way to reclaim the space than deleting the item by hand.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AlternativeDto {
    pub how: String,
    /// Command to run instead (may need an administrator terminal, as `how` says).
    pub command: Option<String>,
}

/// "Can I delete this?" verdict of the Safety Advisor for one node.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AdviceDto {
    #[ts(type = "number")]
    pub generation: u64,
    pub node_id: u32,
    pub level: SafetyLevelDto,
    /// Matched rule (e.g. `hiberfil`, `dev_junk`); `null` when no rule applies.
    pub rule_id: Option<String>,
    pub title: String,
    /// May contain commands in `backticks`.
    pub description: String,
    pub alternative: Option<AlternativeDto>,
    /// Why DiskSieve refuses to move it to Trash; set exactly when `level` is `system`.
    pub protected_reason: Option<String>,
}

impl AdviceDto {
    pub fn new(generation: u64, node_id: NodeId, advice: &Advice) -> Self {
        Self {
            generation,
            node_id,
            level: advice.level.into(),
            rule_id: advice.rule.map(str::to_owned),
            title: advice.title.to_owned(),
            description: advice.description.to_owned(),
            alternative: advice.alternative.map(|a| AlternativeDto {
                how: a.how.to_owned(),
                command: a.command.map(str::to_owned),
            }),
            protected_reason: advice.protected.map(|r| sentence(&r.to_string())),
        }
    }
}

/// What moving the whole Sieve Basket to Trash does with one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SieveStatusDto {
    /// Moved to Trash itself.
    Ready,
    /// Inside another collected folder (`coveredBy`), which takes it along.
    Covered,
    /// Refused by the backend guard; skipped.
    Protected,
}

/// One collected item of the Sieve Basket with its Safety Advisor verdict.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SieveItemDto {
    /// `value` is the allocated size this item frees (0 for hard links counted elsewhere).
    pub node: TreemapNodeDto,
    pub status: SieveStatusDto,
    /// Collected folder that contains this item (only for `covered`).
    pub covered_by: Option<u32>,
    pub advice: AdviceDto,
}

/// The reviewed Sieve Basket. Totals and counts cover only `ready` items.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SieveDto {
    #[ts(type = "number")]
    pub generation: u64,
    /// Items in the order they were collected.
    pub items: Vec<SieveItemDto>,
    /// Collected ids no longer in the tree (already moved to Trash); drop them from the basket.
    pub missing: Vec<u32>,
    #[ts(type = "number")]
    pub allocated_bytes: u64,
    #[ts(type = "number")]
    pub logical_bytes: u64,
    #[ts(type = "number")]
    pub file_count: u64,
    pub safe_count: u32,
    pub review_count: u32,
    pub protected_count: u32,
    /// Ids to pass to `trash_nodes` to empty the basket.
    pub trashable_ids: Vec<u32>,
}

impl SieveDto {
    pub fn new(tree: &ScanTree, generation: u64, review: &SieveReview) -> Self {
        let items = review
            .entries
            .iter()
            .filter_map(|e| {
                let node = tree.get(e.id)?;
                let (status, covered_by) = match e.status {
                    SieveStatus::Ready => (SieveStatusDto::Ready, None),
                    SieveStatus::Covered(by) => (SieveStatusDto::Covered, Some(by)),
                    SieveStatus::Protected => (SieveStatusDto::Protected, None),
                };
                Some(SieveItemDto {
                    node: TreemapNodeDto::from_node(tree, e.id, node, node.contribution().1),
                    status,
                    covered_by,
                    advice: AdviceDto::new(generation, e.id, &e.advice),
                })
            })
            .collect();
        Self {
            generation,
            items,
            missing: review.missing.clone(),
            allocated_bytes: review.allocated_size,
            logical_bytes: review.logical_size,
            file_count: review.file_count,
            safe_count: review.safe,
            review_count: review.review,
            protected_count: review.protected,
            trashable_ids: review.trashable(),
        }
    }
}

/// Capitalizes the first letter and ends with a full stop.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

fn flag_names(flags: NodeFlags) -> Vec<String> {
    const NAMES: [(NodeFlags, &str); 6] = [
        (NodeFlags::HARDLINK, "hardlink"),
        (NodeFlags::CLOUD_ONLINE, "cloudOnline"),
        (NodeFlags::REPARSE, "reparse"),
        (NodeFlags::OTHER_VOLUME, "otherVolume"),
        (NodeFlags::ACCESS_DENIED, "accessDenied"),
        (NodeFlags::NAME_LOSSY, "nameLossy"),
    ];
    NAMES
        .iter()
        .filter(|(flag, _)| flags.contains(*flag))
        .map(|(_, name)| (*name).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use disksieve_core::{build_viewport, EntryMeta, TreeBuilder, ViewportOptions};
    use std::ffi::OsStr;

    #[test]
    fn viewport_maps_items_breadcrumbs_and_other() {
        let dir = EntryMeta {
            flags: NodeFlags::DIR,
            ..EntryMeta::default()
        };
        let file = |size: u64, flags: NodeFlags| EntryMeta {
            logical_size: size,
            allocated_size: size,
            mtime: 1_700_000_000,
            flags,
        };
        let mut b = TreeBuilder::new("/scan", 9, dir);
        let docs = b.push(ROOT, OsStr::new("docs"), dir).unwrap();
        let big = b
            .push(
                docs,
                OsStr::new("Report.PDF"),
                file(5_000, NodeFlags::empty()),
            )
            .unwrap();
        b.push(docs, OsStr::new("small.txt"), file(10, NodeFlags::empty()))
            .unwrap();
        b.push(docs, OsStr::new("link.bin"), file(900, NodeFlags::HARDLINK))
            .unwrap();
        let tree = b.finish(Duration::ZERO);

        let opts = ViewportOptions {
            metric: SizeMetric::Logical,
            max_items: 2,
            ..ViewportOptions::default()
        };
        let view = build_viewport(&tree, docs, &opts).unwrap();
        let dto = ViewportDto::from_viewport(&tree, &view, None, false);

        assert_eq!(dto.generation, 9);
        assert_eq!(dto.current_id, docs);
        assert_eq!(dto.metric, SizeMetricDto::Logical);
        assert_eq!(
            dto.breadcrumbs
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            vec!["/scan", "docs"]
        );
        assert_eq!(dto.nodes.len(), 3, "two items + Other");
        let first = &dto.nodes[0];
        assert_eq!(first.id, Some(big));
        assert_eq!(first.extension.as_deref(), Some("pdf"));
        assert!(first.path.ends_with("Report.PDF"), "{}", first.path);
        // The hard link has value 0, so it is merged into Other.
        let other = dto.nodes.last().unwrap();
        assert_eq!(other.id, None);
        assert_eq!(other.other_count, Some(1));
        assert_eq!(dto.total_value, 5_010);

        let json = serde_json::to_value(&dto).unwrap();
        assert!(json["nodes"][2]["id"].is_null(), "{json}");
        assert_eq!(json["metric"], "logical");
        assert!(json["nodes"][0].get("allocatedSize").is_some(), "{json}");
        assert!(json["deltaBytes"].is_null(), "no previous scan: {json}");
        assert_eq!(dto.removed_count, 0);
    }

    #[test]
    fn delta_viewport_lists_changes_and_removed_items() {
        use disksieve_core::Snapshot;
        const MIB: u64 = 1024 * 1024;
        let dir = EntryMeta {
            flags: NodeFlags::DIR,
            ..EntryMeta::default()
        };
        let file = |size: u64| EntryMeta {
            logical_size: size,
            allocated_size: size,
            mtime: 1_700_000_000,
            flags: NodeFlags::empty(),
        };
        let build = |files: &[(&str, u64)]| {
            let mut b = TreeBuilder::new("/scan", 1, dir);
            let docs = b.push(ROOT, OsStr::new("docs"), dir).unwrap();
            for &(name, size) in files {
                b.push(docs, OsStr::new(name), file(size)).unwrap();
            }
            (b.finish(Duration::ZERO), docs)
        };
        let (before, _) = build(&[("old.bin", 2 * MIB), ("grow.bin", 2 * MIB)]);
        let (after, docs) = build(&[("grow.bin", 5 * MIB), ("new.bin", 4 * MIB)]);
        let delta = Delta::compute(&after, &Snapshot::capture(&before));

        let opts = ViewportOptions {
            delta: Some(delta.as_slice()),
            ..ViewportOptions::default()
        };
        let view = build_viewport(&after, docs, &opts).unwrap();
        let dto = ViewportDto::from_viewport(&after, &view, Some(&delta), true);

        assert!(dto.delta_mode);
        assert_eq!(dto.delta_bytes, Some(5 * MIB as i64));
        let rows: Vec<_> = dto
            .nodes
            .iter()
            .map(|n| (n.name.as_str(), n.delta_bytes))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("new.bin", Some(4 * MIB as i64)),
                ("grow.bin", Some(3 * MIB as i64))
            ]
        );
        assert_eq!(dto.removed_count, 1);
        let removed = &dto.removed[0];
        assert_eq!(removed.name, "old.bin");
        assert_eq!(removed.allocated_size, 2 * MIB);
        assert!(!removed.is_dir);
        assert!(removed.path.ends_with("old.bin"), "{}", removed.path);
    }

    #[test]
    fn duplicates_drop_trashed_copies_and_recount_waste() {
        use disksieve_core::{Candidate, DuplicateGroup, DuplicateReport};
        let dir = EntryMeta {
            flags: NodeFlags::DIR,
            ..EntryMeta::default()
        };
        let file = |size: u64| EntryMeta {
            logical_size: size,
            allocated_size: size,
            mtime: 1_700_000_000,
            flags: NodeFlags::empty(),
        };
        let mut b = TreeBuilder::new("/scan", 4, dir);
        let a = b.push(ROOT, OsStr::new("a"), dir).unwrap();
        let x1 = b.push(a, OsStr::new("x.iso"), file(100)).unwrap();
        let x2 = b.push(ROOT, OsStr::new("x.iso"), file(100)).unwrap();
        let x3 = b.push(ROOT, OsStr::new("x (1).iso"), file(100)).unwrap();
        let y1 = b.push(a, OsStr::new("y.zip"), file(500)).unwrap();
        let y2 = b.push(ROOT, OsStr::new("y.zip"), file(500)).unwrap();
        let mut tree = b.finish(Duration::ZERO);
        let candidate = |id, size, mtime| Candidate {
            id,
            size,
            mtime,
            path: tree.resolve_path(id).unwrap(),
        };
        let report = DuplicateReport {
            groups: vec![
                DuplicateGroup {
                    size: 500,
                    hash: [0xAB; 32],
                    copies: vec![candidate(y1, 500, 1), candidate(y2, 500, 2)],
                },
                DuplicateGroup {
                    size: 100,
                    hash: [0x01; 32],
                    copies: vec![
                        candidate(x1, 100, 1),
                        candidate(x2, 100, 2),
                        candidate(x3, 100, 3),
                    ],
                },
            ],
            files_checked: 5,
            bytes_read: 1_000,
            skipped: 1,
        };

        let dto = DuplicatesDto::new(&tree, &report, Duration::from_millis(42));
        assert_eq!(
            (dto.group_count, dto.copy_count, dto.wasted_bytes),
            (2, 5, 700)
        );
        assert_eq!(dto.groups[0].hash, "ab".repeat(32));
        assert_eq!(dto.groups[0].copies[0].folder_id, a);
        assert_eq!(dto.duration_ms, 42);

        // Trashing folder `a` removes y1 (its group is gone) and x1 (two copies left).
        tree.mark_trashed(a).unwrap();
        let dto = DuplicatesDto::new(&tree, &report, Duration::ZERO);
        assert_eq!(
            (dto.group_count, dto.copy_count, dto.wasted_bytes),
            (1, 2, 100)
        );
        let ids: Vec<u32> = dto.groups[0].copies.iter().map(|c| c.node_id).collect();
        assert_eq!(ids, [x2, x3]);
        assert_eq!(dto.groups[0].name, "x.iso");

        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["groups"][0]["wastedBytes"], 100, "{json}");
        assert_eq!(json["groups"][0]["copies"][0]["nodeId"], x2, "{json}");
    }

    #[test]
    fn changes_map_kinds_paths_and_totals() {
        use disksieve_core::{list_changes, Snapshot};
        const MIB: u64 = 1024 * 1024;
        let dir = EntryMeta {
            flags: NodeFlags::DIR,
            ..EntryMeta::default()
        };
        let file = |size: u64| EntryMeta {
            logical_size: size,
            allocated_size: size,
            mtime: 1_700_000_000,
            flags: NodeFlags::empty(),
        };
        let build = |files: &[(&str, u64)]| {
            let mut b = TreeBuilder::new("/scan", 3, dir);
            let docs = b.push(ROOT, OsStr::new("docs"), dir).unwrap();
            for &(name, size) in files {
                b.push(docs, OsStr::new(name), file(size)).unwrap();
            }
            (b.finish(Duration::ZERO), docs)
        };
        let (before, _) = build(&[("old.bin", 2 * MIB), ("grow.bin", 2 * MIB), ("a.txt", 10)]);
        let (after, docs) = build(&[("grow.bin", 5 * MIB), ("a.txt", 30)]);
        let delta = Delta::compute(&after, &Snapshot::capture(&before));
        let list = list_changes(&after, &delta, None, 500);
        let dto = ChangesDto::new(&after, &delta, None, &list);

        assert_eq!(dto.generation, 3);
        assert_eq!(dto.matching, 3);
        let grew = &dto.changes[0];
        assert_eq!(grew.kind, ChangeKindDto::Grew);
        assert_eq!(grew.folder_id, docs);
        assert_eq!(
            (grew.before_size, grew.after_size),
            (Some(2 * MIB), Some(5 * MIB))
        );
        let removed = &dto.changes[1];
        assert_eq!(removed.kind, ChangeKindDto::Removed);
        assert_eq!(removed.node_id, None);
        assert!(removed.path.ends_with("old.bin"), "{}", removed.path);
        let small = &dto.changes[2];
        assert_eq!(small.kind, ChangeKindDto::SmallFiles);
        assert_eq!((small.bytes, small.folder_id), (20, docs));
        assert_eq!(dto.net, dto.changes.iter().map(|c| c.bytes).sum::<i64>());
        assert_eq!(dto.totals.len(), 5);

        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["changes"][2]["kind"], "smallFiles", "{json}");
        assert!(json["changes"][1]["afterSize"].is_null(), "{json}");
    }

    #[test]
    fn advice_maps_level_texts_and_refusal() {
        use disksieve_core::{Advice, Alternative, Refusal, SafetyLevel};
        let advice = Advice {
            level: SafetyLevel::System,
            rule: Some("hiberfil"),
            title: "Hibernation file",
            description: "d",
            alternative: Some(Alternative {
                how: "h",
                command: Some("powercfg /hibernate off"),
            }),
            protected: Some(Refusal::SystemFile),
        };
        let dto = AdviceDto::new(3, 7, &advice);
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["level"], "system");
        assert_eq!(json["ruleId"], "hiberfil");
        assert_eq!(json["nodeId"], 7);
        assert_eq!(json["alternative"]["command"], "powercfg /hibernate off");
        assert_eq!(
            json["protectedReason"],
            "System file — managed by the operating system."
        );
        let none = AdviceDto::new(
            3,
            7,
            &Advice {
                level: SafetyLevel::Review,
                rule: None,
                alternative: None,
                protected: None,
                ..advice
            },
        );
        let json = serde_json::to_value(&none).unwrap();
        assert!(json["ruleId"].is_null() && json["alternative"].is_null());
        assert!(json["protectedReason"].is_null());
    }

    #[test]
    fn flag_names_are_camel_case() {
        let names = flag_names(NodeFlags::DIR | NodeFlags::CLOUD_ONLINE | NodeFlags::ACCESS_DENIED);
        assert_eq!(names, vec!["cloudOnline", "accessDenied"]);
    }

    #[test]
    fn metric_deserializes_from_lowercase() {
        let m: SizeMetricDto = serde_json::from_str("\"allocated\"").unwrap();
        assert_eq!(m, SizeMetricDto::Allocated);
    }

    #[test]
    fn scan_summary_counts_files_dirs_and_sizes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub").join("a.bin"), [0u8; 1000]).unwrap();
        std::fs::write(tmp.path().join("b.bin"), [0u8; 24]).unwrap();
        let opts = disksieve_core::ScanOptions {
            generation: 5,
            ..Default::default()
        };
        let tree = disksieve_core::scan(
            tmp.path(),
            &opts,
            &disksieve_core::CancelToken::new(),
            |_| {},
        )
        .unwrap();
        let summary = ScanSummaryDto::from_tree(&tree, None);
        assert_eq!(summary.generation, 5);
        assert!(summary.volume.is_none());
        assert!(summary.skipped_samples.is_empty());
        assert_eq!(summary.file_count, 2);
        assert_eq!(summary.dir_count, 2, "root + sub");
        assert_eq!(summary.logical_bytes, 1024);
        assert!(summary.allocated_bytes >= 1024);

        let json = serde_json::to_value(&summary).unwrap();
        assert!(json.get("rootPath").is_some(), "{json}");
        assert!(json.get("durationMs").is_some(), "{json}");
        assert!(json["volume"].is_null(), "{json}");
    }

    #[test]
    fn space_balance_maps_to_camel_case() {
        let dto = SpaceBalanceDto::from(SpaceBalance::new(1000, 300, 650));
        assert_eq!(
            (dto.used_bytes, dto.free_bytes, dto.unaccounted_bytes),
            (700, 300, 50)
        );
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["unaccountedBytes"], 50, "{json}");
        assert_eq!(json["scannedBytes"], 650, "{json}");
    }

    #[test]
    fn trash_report_sums_successes_and_classifies_errors() {
        use disksieve_core::{Freed, Refusal};
        let ok = |id, allocated| TrashOutcome {
            id,
            path: format!("/p/{id}"),
            result: Ok(Freed {
                logical: allocated / 2,
                allocated,
                files: 1,
            }),
        };
        let outcomes = [
            ok(1, 4096),
            TrashOutcome {
                id: 2,
                path: "/p/2".into(),
                result: Err(Refusal::SystemLocation.into()),
            },
            TrashOutcome {
                id: 3,
                path: "/p/3".into(),
                result: Err(TrashError::Changed("size differs")),
            },
            ok(4, 8192),
        ];
        let report = TrashReportDto::new(5, &outcomes, true);
        assert_eq!(
            (
                report.allocated_bytes,
                report.logical_bytes,
                report.file_count
            ),
            (12_288, 6_144, 2)
        );
        assert_eq!(
            report.items[1].error_kind,
            Some(TrashErrorKindDto::Protected)
        );
        assert_eq!(report.items[2].error_kind, Some(TrashErrorKindDto::Changed));
        assert_eq!(report.items[2].allocated_bytes, 0);

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["undoAvailable"], true, "{json}");
        assert_eq!(json["items"][1]["errorKind"], "protected", "{json}");
        assert!(json["items"][0]["error"].is_null(), "{json}");
    }

    #[test]
    fn search_result_maps_hits_with_their_parent_folder() {
        let dir = EntryMeta {
            flags: NodeFlags::DIR,
            ..EntryMeta::default()
        };
        let file = |size: u64| EntryMeta {
            logical_size: size,
            allocated_size: size,
            mtime: 1_700_000_000,
            flags: NodeFlags::empty(),
        };
        let mut b = TreeBuilder::new("/scan", 4, dir);
        let docs = b.push(ROOT, OsStr::new("docs"), dir).unwrap();
        let report = b.push(docs, OsStr::new("Report.pdf"), file(5_000)).unwrap();
        b.push(ROOT, OsStr::new("report-old.pdf"), file(10))
            .unwrap();
        let tree = b.finish(Duration::ZERO);

        let result = disksieve_core::search_nodes(&tree, "REPORT", SizeMetric::Logical, 50);
        let dto = SearchResultDto::new(&tree, "REPORT", SizeMetric::Logical, &result);
        assert_eq!(dto.generation, 4);
        assert_eq!(dto.total_matches, 2);
        assert_eq!(dto.hits[0].node.id, Some(report));
        assert_eq!(dto.hits[0].node.value, 5_000);
        assert_eq!(dto.hits[0].parent_id, docs);
        assert_eq!(dto.hits[1].parent_id, ROOT);
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["totalMatches"], 2);
        assert_eq!(json["hits"][0]["parentId"], docs);
        assert_eq!(json["hits"][0]["node"]["name"], "Report.pdf");
    }

    #[test]
    fn dev_junk_maps_projects_artifacts_and_viewport_marks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for (path, bytes) in [
            ("code/web/package.json", 10),
            ("code/web/node_modules/x.js", 30_000),
            ("code/py/pyproject.toml", 10),
            ("code/py/pkg/__pycache__/m.pyc", 500),
        ] {
            let p = root.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, vec![0u8; bytes]).unwrap();
        }
        let opts = disksieve_core::ScanOptions {
            generation: 3,
            ..Default::default()
        };
        let mut tree =
            disksieve_core::scan(root, &opts, &disksieve_core::CancelToken::new(), |_| {}).unwrap();
        disksieve_core::detect_junk(&mut tree);

        let dto = DevJunkDto::new(&tree, &disksieve_core::junk_projects(&tree));
        assert_eq!(dto.generation, 3);
        assert_eq!(dto.artifact_count, 2);
        assert_eq!(dto.projects[0].name, "web", "largest first");
        let py = &dto.projects[1];
        assert_eq!(py.name, "py");
        assert_eq!(py.artifacts[0].rel_path, "pkg/__pycache__");
        assert_eq!(py.artifacts[0].kind, "python_cache");
        assert_eq!(
            dto.logical_bytes,
            dto.projects.iter().map(|p| p.logical_bytes).sum::<u64>()
        );
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            json["projects"][1]["artifacts"][0]["relPath"],
            "pkg/__pycache__"
        );

        let code = tree.child_by_name(ROOT, "code").unwrap();
        let view = build_viewport(&tree, code, &ViewportOptions::default()).unwrap();
        let tiles = ViewportDto::from_viewport(&tree, &view, None, false).nodes;
        assert!(tiles
            .iter()
            .all(|t| t.flags.contains(&"containsJunk".to_owned())));
        let web = tree.child_by_name(code, "web").unwrap();
        let view = build_viewport(&tree, web, &ViewportOptions::default()).unwrap();
        let nm = ViewportDto::from_viewport(&tree, &view, None, false)
            .nodes
            .into_iter()
            .find(|t| t.name == "node_modules")
            .unwrap();
        assert_eq!(nm.junk_kind.as_deref(), Some("node_modules"));
        assert!(nm.flags.is_empty());
    }

    #[test]
    fn app_info_serializes_to_camel_case() {
        let json = serde_json::to_value(AppInfoDto::current()).unwrap();
        assert!(json.get("coreVersion").is_some(), "{json}");
        assert!(json.get("core_version").is_none(), "{json}");
    }
}
