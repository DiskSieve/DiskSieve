//! IPC data transfer objects — the only types that cross the Rust ↔ WebView boundary.
//!
//! Every DTO derives `TS` and is exported to `src/types/generated/` (see `.cargo/config.toml`).
//! Regenerate the TypeScript bindings with `npm run gen:types`; never edit them by hand.
//! Rules: `#[serde(rename_all = "camelCase")]`, `#[ts(export)]`, and `#[ts(type = "number")]`
//! on 64-bit integers (values stay below 2^53).

use disksieve_core::{
    contains_junk, JunkKind, JunkProject, Node, NodeFlags, NodeId, OtherBucket, ScanProgress,
    ScanTree, SizeMetric, SpaceBalance, TrashError, TrashOutcome, Viewport, ROOT,
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
    /// `null` while Delta Mode is not available.
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
    pub nodes: Vec<TreemapNodeDto>,
}

impl ViewportDto {
    pub fn from_viewport(tree: &ScanTree, view: &Viewport) -> Self {
        let current_path = tree.display_path(view.current);
        let mut nodes: Vec<TreemapNodeDto> = view
            .items
            .iter()
            .filter_map(|item| {
                let node = tree.get(item.id)?;
                Some(TreemapNodeDto::from_node(tree, item.id, node, item.value))
            })
            .collect();
        if let Some(bucket) = &view.other {
            nodes.push(TreemapNodeDto::other(&current_path, bucket));
        }
        Self {
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
        };
        let view = build_viewport(&tree, docs, &opts).unwrap();
        let dto = ViewportDto::from_viewport(&tree, &view);

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
        let tiles = ViewportDto::from_viewport(&tree, &view).nodes;
        assert!(tiles
            .iter()
            .all(|t| t.flags.contains(&"containsJunk".to_owned())));
        let web = tree.child_by_name(code, "web").unwrap();
        let view = build_viewport(&tree, web, &ViewportOptions::default()).unwrap();
        let nm = ViewportDto::from_viewport(&tree, &view)
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
