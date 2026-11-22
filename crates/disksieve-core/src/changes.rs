//! Changes view (roadmap step 6.3): everything that changed since the previous scan as one
//! flat list, like `git diff --stat`.
//!
//! [`list_changes`] walks the tree once with the [`Delta`] overlay and emits:
//!
//! * **Added** — the topmost new folder of a branch, or a new file ≥ 1 MiB;
//! * **Removed** — the topmost removed item of a branch (with its name from the snapshot);
//! * **Grew / Shrank** — a file ≥ 1 MiB stored in both scans whose size changed;
//! * **Small files** — per folder, the change of the files below 1 MiB directly in it
//!   (snapshots do not store them one by one).
//!
//! Nothing is counted twice, so the entries add up to the change of the scan root. Pure tree
//! logic — no filesystem access.

use crate::snapshot::Delta;
use crate::tree::{NodeId, ScanTree, ROOT};

/// Default number of entries returned by [`list_changes`].
pub const DEFAULT_CHANGES_LIMIT: usize = 500;

/// Kind of one change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
    Removed,
    Grew,
    Shrank,
    SmallFiles,
}

impl ChangeKind {
    pub const ALL: [Self; 5] = [
        Self::Added,
        Self::Removed,
        Self::Grew,
        Self::Shrank,
        Self::SmallFiles,
    ];

    fn index(self) -> usize {
        self as usize
    }
}

/// What a change refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeItem {
    /// An item of the current scan (added, grew or shrank).
    Node(NodeId),
    /// Index into [`Delta::removed`].
    Removed(usize),
    /// Files below 1 MiB directly in this folder.
    SmallFiles(NodeId),
}

/// One line of the Changes view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub item: ChangeItem,
    /// Change in bytes on disk (negative for shrank and removed).
    pub bytes: i64,
}

/// Number and sum of the changes of one kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KindTotal {
    pub count: u32,
    pub bytes: i64,
}

/// Result of [`list_changes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeList {
    /// The largest changes (by absolute size) of the requested kind, largest first.
    pub changes: Vec<Change>,
    /// Totals of all changes, by [`ChangeKind`] (in the order of [`ChangeKind::ALL`]).
    pub totals: [KindTotal; 5],
    /// Number of changes matching the filter (may exceed `changes.len()`).
    pub matching: u32,
    /// Sum of all growth.
    pub grown: u64,
    /// Sum of all shrinking and removal (positive).
    pub shrunk: u64,
}

impl ChangeList {
    /// Totals of one kind.
    pub fn total(&self, kind: ChangeKind) -> KindTotal {
        self.totals[kind.index()]
    }

    /// Change of the whole scan (`grown − shrunk`).
    pub fn net(&self) -> i64 {
        signed(self.grown).saturating_sub(signed(self.shrunk))
    }
}

fn signed(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Every change of `tree` since the snapshot behind `delta`, with totals; the `limit` largest
/// changes of `kind` (all kinds when `None`) are listed.
pub fn list_changes(
    tree: &ScanTree,
    delta: &Delta,
    kind: Option<ChangeKind>,
    limit: usize,
) -> ChangeList {
    let all = collect(tree, delta);

    let mut totals = [KindTotal::default(); 5];
    let (mut grown, mut shrunk) = (0u64, 0u64);
    for c in &all {
        let t = &mut totals[c.kind.index()];
        t.count = t.count.saturating_add(1);
        t.bytes = t.bytes.saturating_add(c.bytes);
        if c.bytes >= 0 {
            grown = grown.saturating_add(c.bytes.unsigned_abs());
        } else {
            shrunk = shrunk.saturating_add(c.bytes.unsigned_abs());
        }
    }

    let mut changes: Vec<Change> = all
        .into_iter()
        .filter(|c| kind.is_none_or(|k| c.kind == k))
        .collect();
    let matching = u32::try_from(changes.len()).unwrap_or(u32::MAX);
    // Largest first; on a tie growth before shrinking, so the order is stable.
    let by_size = |a: &Change, b: &Change| {
        b.bytes
            .unsigned_abs()
            .cmp(&a.bytes.unsigned_abs())
            .then(b.bytes.cmp(&a.bytes))
    };
    if limit < changes.len() {
        if limit > 0 {
            changes.select_nth_unstable_by(limit - 1, by_size);
        }
        changes.truncate(limit);
    }
    changes.sort_by(by_size);

    ChangeList {
        changes,
        totals,
        matching,
        grown,
        shrunk,
    }
}

/// All changes, unsorted.
fn collect(tree: &ScanTree, delta: &Delta) -> Vec<Change> {
    let bytes = |id: NodeId| delta.bytes(id).unwrap_or(0);
    let mut out = Vec::new();
    let mut stack = vec![ROOT];
    while let Some(id) = stack.pop() {
        let Some(node) = tree.get(id).filter(|n| !n.is_trashed()) else {
            continue;
        };
        let change = bytes(id);
        if delta.before(id).is_none() {
            // New: a folder (with everything inside) or a large file. New small files have
            // no change of their own; they show in their folder's "small files".
            if change != 0 {
                out.push(Change {
                    kind: ChangeKind::Added,
                    item: ChangeItem::Node(id),
                    bytes: change,
                });
            }
            continue;
        }
        if !node.is_dir() {
            if change != 0 {
                out.push(Change {
                    kind: if change > 0 {
                        ChangeKind::Grew
                    } else {
                        ChangeKind::Shrank
                    },
                    item: ChangeItem::Node(id),
                    bytes: change,
                });
            }
            continue;
        }

        // Folder in both scans: its change is spread over its children, its removed items
        // and its small files (whatever is left).
        let mut rest = change;
        for child in tree.children(id) {
            if tree.get(child).is_some_and(|c| !c.is_trashed()) {
                rest = rest.saturating_sub(bytes(child));
                stack.push(child);
            }
        }
        let first = delta.removed().partition_point(|r| r.parent < id);
        for (offset, r) in delta.removed_in(id).iter().enumerate() {
            let removed = -signed(r.allocated);
            rest = rest.saturating_sub(removed);
            out.push(Change {
                kind: ChangeKind::Removed,
                item: ChangeItem::Removed(first + offset),
                bytes: removed,
            });
        }
        if rest != 0 {
            out.push(Change {
                kind: ChangeKind::SmallFiles,
                item: ChangeItem::SmallFiles(id),
                bytes: rest,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Snapshot;
    use crate::tree::{EntryMeta, NodeFlags, TreeBuilder};
    use std::ffi::OsStr;
    use std::time::Duration;

    const MIB: u64 = 1024 * 1024;

    fn dir() -> EntryMeta {
        EntryMeta {
            flags: NodeFlags::DIR,
            ..EntryMeta::default()
        }
    }

    fn file(size: u64) -> EntryMeta {
        EntryMeta {
            logical_size: size,
            allocated_size: size,
            ..EntryMeta::default()
        }
    }

    /// Builds `/data` from `(folder, name, size)`; folder `""` is the root, folders are
    /// created on first use (one level deep is enough here).
    fn build(files: &[(&str, &str, u64)]) -> ScanTree {
        let mut b = TreeBuilder::new("/data", 1, dir());
        let mut folders: Vec<(&str, NodeId)> = Vec::new();
        for &(folder, name, size) in files {
            let parent = if folder.is_empty() {
                ROOT
            } else if let Some(&(_, id)) = folders.iter().find(|(f, _)| *f == folder) {
                id
            } else {
                let id = b.push(ROOT, OsStr::new(folder), dir()).unwrap();
                folders.push((folder, id));
                id
            };
            if !name.is_empty() {
                b.push(parent, OsStr::new(name), file(size)).unwrap();
            }
        }
        b.finish(Duration::ZERO)
    }

    fn named(tree: &ScanTree, delta: &Delta, c: &Change) -> String {
        match c.item {
            ChangeItem::Node(id) => tree.name(id).to_owned(),
            ChangeItem::Removed(i) => delta.removed()[i].name.clone(),
            ChangeItem::SmallFiles(id) => format!("{}/<small>", tree.name(id)),
        }
    }

    #[test]
    fn lists_every_kind_and_adds_up_to_the_root_change() {
        let before = build(&[
            ("docs", "grow.bin", 2 * MIB),
            ("docs", "shrink.bin", 6 * MIB),
            ("docs", "gone.bin", 3 * MIB),
            ("docs", "note.txt", 1000),
            ("old", "a.bin", 4 * MIB),
        ]);
        let after = build(&[
            ("docs", "grow.bin", 5 * MIB),
            ("docs", "shrink.bin", MIB),
            ("docs", "note.txt", 5000),
            ("docs", "tiny.txt", 300),
            ("new", "b.bin", 2 * MIB),
            ("", "big.iso", 7 * MIB),
        ]);
        let delta = Delta::compute(&after, &Snapshot::capture(&before));
        let list = list_changes(&after, &delta, None, DEFAULT_CHANGES_LIMIT);

        let rows: Vec<(String, ChangeKind, i64)> = list
            .changes
            .iter()
            .map(|c| (named(&after, &delta, c), c.kind, c.bytes))
            .collect();
        let m = MIB as i64;
        assert_eq!(
            rows,
            vec![
                ("big.iso".into(), ChangeKind::Added, 7 * m),
                ("shrink.bin".into(), ChangeKind::Shrank, -5 * m),
                ("old".into(), ChangeKind::Removed, -4 * m),
                ("grow.bin".into(), ChangeKind::Grew, 3 * m),
                ("gone.bin".into(), ChangeKind::Removed, -3 * m),
                ("new".into(), ChangeKind::Added, 2 * m),
                ("docs/<small>".into(), ChangeKind::SmallFiles, 4300),
            ]
        );
        let sum: i64 = list.changes.iter().map(|c| c.bytes).sum();
        assert_eq!(sum, delta.bytes(ROOT).unwrap(), "entries add up");
        assert_eq!(list.net(), sum);
        assert_eq!(list.grown, (12 * MIB) + 4300);
        assert_eq!(list.shrunk, 12 * MIB);
        assert_eq!(
            list.total(ChangeKind::Removed),
            KindTotal {
                count: 2,
                bytes: -7 * m
            }
        );
        assert_eq!(list.matching, 7);
    }

    #[test]
    fn filter_and_limit_keep_totals_of_everything() {
        let before = build(&[("", "a.bin", 2 * MIB), ("", "b.bin", 2 * MIB)]);
        let after = build(&[
            ("", "a.bin", 3 * MIB),
            ("", "b.bin", 6 * MIB),
            ("", "c.bin", 9 * MIB),
        ]);
        let delta = Delta::compute(&after, &Snapshot::capture(&before));

        let grew = list_changes(&after, &delta, Some(ChangeKind::Grew), 1);
        assert_eq!(grew.matching, 2);
        assert_eq!(grew.changes.len(), 1);
        assert_eq!(named(&after, &delta, &grew.changes[0]), "b.bin");
        assert_eq!(grew.total(ChangeKind::Added).bytes, 9 * MIB as i64);
        assert_eq!(grew.grown, 14 * MIB);

        assert!(list_changes(&after, &delta, None, 0).changes.is_empty());
    }

    #[test]
    fn trashed_items_are_removed_and_unchanged_trees_are_empty() {
        let tree = build(&[("docs", "a.bin", 2 * MIB), ("docs", "b.txt", 10)]);
        let snapshot = Snapshot::capture(&tree);
        let list = list_changes(
            &tree,
            &Delta::compute(&tree, &snapshot),
            None,
            DEFAULT_CHANGES_LIMIT,
        );
        assert!(list.changes.is_empty());
        assert_eq!((list.grown, list.shrunk), (0, 0));

        let mut tree = tree;
        let docs = tree.child_by_name(ROOT, "docs").unwrap();
        tree.mark_trashed(docs).unwrap();
        let delta = Delta::compute(&tree, &snapshot);
        let list = list_changes(&tree, &delta, None, DEFAULT_CHANGES_LIMIT);
        let rows: Vec<_> = list
            .changes
            .iter()
            .map(|c| (named(&tree, &delta, c), c.kind))
            .collect();
        assert_eq!(rows, vec![("docs".to_owned(), ChangeKind::Removed)]);
        assert_eq!(list.net(), delta.bytes(ROOT).unwrap());
    }
}
