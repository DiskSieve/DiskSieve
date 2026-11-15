//! IPC data transfer objects — the only types that cross the Rust ↔ WebView boundary.
//!
//! Every DTO derives `TS` and is exported to `src/types/generated/` (see `.cargo/config.toml`).
//! Regenerate the TypeScript bindings with `npm run gen:types`; never edit them by hand.
//! Rules: `#[serde(rename_all = "camelCase")]`, `#[ts(export)]`, and `#[ts(type = "number")]`
//! on 64-bit integers (values stay below 2^53).

use disksieve_core::{
    Node, NodeFlags, NodeId, OtherBucket, ScanProgress, ScanTree, SizeMetric, Viewport, ROOT,
};
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
}

impl ScanProgressDto {
    pub fn new(generation: u64, p: &ScanProgress) -> Self {
        Self {
            generation,
            files: p.files,
            dirs: p.dirs,
            allocated_bytes: p.allocated_bytes,
            skipped_entries: p.skipped_entries,
            current_dir: p.current_dir.to_string_lossy().into_owned(),
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
    #[ts(type = "number")]
    pub duration_ms: u64,
}

impl ScanSummaryDto {
    pub fn from_tree(tree: &ScanTree) -> Self {
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
            duration_ms: u64::try_from(tree.stats().duration.as_millis()).unwrap_or(u64::MAX),
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
    /// Dev-junk kind, e.g. `node_modules` (filled in by Dev Junk detection, v0.1 step 4).
    pub junk_kind: Option<String>,
    /// e.g. `["hardlink", "cloudOnline", "accessDenied"]`.
    pub flags: Vec<String>,
    /// Number of merged children (only for "Other").
    pub other_count: Option<u32>,
}

impl TreemapNodeDto {
    fn from_node(tree: &ScanTree, id: NodeId, node: &Node, value: u64) -> Self {
        let ext = tree.extension(id);
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
            junk_kind: None,
            flags: flag_names(node.flags),
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
    use std::time::Duration;

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
        let summary = ScanSummaryDto::from_tree(&tree);
        assert_eq!(summary.generation, 5);
        assert_eq!(summary.file_count, 2);
        assert_eq!(summary.dir_count, 2, "root + sub");
        assert_eq!(summary.logical_bytes, 1024);
        assert!(summary.allocated_bytes >= 1024);

        let json = serde_json::to_value(&summary).unwrap();
        assert!(json.get("rootPath").is_some(), "{json}");
        assert!(json.get("durationMs").is_some(), "{json}");
    }

    #[test]
    fn app_info_serializes_to_camel_case() {
        let json = serde_json::to_value(AppInfoDto::current()).unwrap();
        assert!(json.get("coreVersion").is_some(), "{json}");
        assert!(json.get("core_version").is_none(), "{json}");
    }
}
