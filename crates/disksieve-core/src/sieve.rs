//! Sieve Basket (roadmap step 5.2): items collected from several views (treemap, Dev Junk,
//! later duplicates) reviewed together before they go to Trash in one go.
//!
//! [`review_sieve`] gives every collected node its Safety Advisor verdict and decides what one
//! "Move to Trash" of the whole basket would do with it: move it, take it along with a
//! collected folder above it (counted once), or skip it because the delete guard refuses it.
//! Nodes that no longer exist in the tree (moved to Trash meanwhile) are reported as missing.
//!
//! Pure tree logic — no filesystem access. Deleting still goes through
//! [`trash_nodes`](crate::safety::trash_nodes), which checks every node again.

use std::collections::HashSet;

use crate::safety::Guard;
use crate::safety_advisor::{Advice, Advisor, SafetyLevel};
use crate::tree::{NodeId, ScanTree};

/// What moving the whole basket to Trash does with one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SieveStatus {
    /// Moved to Trash itself.
    Ready,
    /// Inside the collected folder `NodeId`, which takes it along; not counted again.
    Covered(NodeId),
    /// Refused by the delete guard (`advice.level` is `System`); skipped.
    Protected,
}

/// One collected node with its verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SieveEntry {
    pub id: NodeId,
    pub status: SieveStatus,
    pub advice: Advice,
}

/// The reviewed basket. Totals and counts cover only [`SieveStatus::Ready`] entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SieveReview {
    /// Entries in the order they were collected (duplicates removed).
    pub entries: Vec<SieveEntry>,
    /// Collected ids no longer in the tree (unknown, or moved to Trash with an ancestor).
    pub missing: Vec<NodeId>,
    pub logical_size: u64,
    pub allocated_size: u64,
    pub file_count: u64,
    /// Ready entries the advisor considers safe.
    pub safe: u32,
    /// Ready entries the advisor wants reviewed first.
    pub review: u32,
    /// Entries the guard refuses.
    pub protected: u32,
}

impl SieveReview {
    /// Ids to pass to `trash_nodes` for the whole basket: the ready entries.
    pub fn trashable(&self) -> Vec<NodeId> {
        self.entries
            .iter()
            .filter(|e| e.status == SieveStatus::Ready)
            .map(|e| e.id)
            .collect()
    }
}

/// Reviews the collected nodes `ids` of `tree` with `advisor` and the delete `guard`.
pub fn review_sieve(
    tree: &ScanTree,
    ids: &[NodeId],
    advisor: &Advisor,
    guard: &Guard,
) -> SieveReview {
    let mut review = SieveReview::default();
    let mut seen = HashSet::with_capacity(ids.len());
    let mut judged = Vec::with_capacity(ids.len());
    for &id in ids {
        if !seen.insert(id) {
            continue;
        }
        let alive = tree.get(id).is_some_and(|n| !n.is_trashed())
            && !tree
                .ancestors(id)
                .any(|a| tree.get(a).is_some_and(|n| n.is_trashed()));
        match advisor.advise(tree, id, guard) {
            Ok(advice) if alive => judged.push((id, advice)),
            _ => review.missing.push(id),
        }
    }

    // Folders that the basket moves to Trash take everything collected inside them along.
    let movable: HashSet<NodeId> = judged
        .iter()
        .filter(|(_, a)| a.level != SafetyLevel::System)
        .map(|(id, _)| *id)
        .collect();
    for (id, advice) in judged {
        let status = match tree.ancestors(id).find(|a| movable.contains(a)) {
            Some(parent) => SieveStatus::Covered(parent),
            None if advice.level == SafetyLevel::System => SieveStatus::Protected,
            None => SieveStatus::Ready,
        };
        match status {
            SieveStatus::Ready => {
                // Hard links already counted elsewhere free nothing (contribution 0).
                let (logical, allocated, files) =
                    tree.get(id).map(|n| n.contribution()).unwrap_or_default();
                review.logical_size += logical;
                review.allocated_size += allocated;
                review.file_count += u64::from(files);
                if advice.level == SafetyLevel::Safe {
                    review.safe += 1;
                } else {
                    review.review += 1;
                }
            }
            SieveStatus::Protected => review.protected += 1,
            SieveStatus::Covered(_) => {}
        }
        review.entries.push(SieveEntry { id, status, advice });
    }
    review
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::Refusal;
    use crate::safety_advisor::Platform;
    use crate::tree::{EntryMeta, NodeFlags, TreeBuilder, ROOT};
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::time::Duration;

    fn dir() -> EntryMeta {
        EntryMeta {
            logical_size: 0,
            allocated_size: 0,
            mtime: 0,
            flags: NodeFlags::DIR,
        }
    }

    fn file(size: u64) -> EntryMeta {
        EntryMeta {
            logical_size: size,
            allocated_size: size.next_multiple_of(4096),
            mtime: 0,
            flags: NodeFlags::empty(),
        }
    }

    struct Fixture {
        tree: ScanTree,
        home: PathBuf,
        cache: NodeId,
        pip: NodeId,
        wheel: NodeId,
        notes: NodeId,
        app: NodeId,
    }

    /// Home folder tree (paths only — nothing is read from disk):
    /// `.cache/pip/a.whl` (10 000 B), `notes.txt` (100 B), `app/` (protected as DiskSieve's own).
    fn fixture() -> Fixture {
        let home = std::env::temp_dir().join("disksieve-sieve-home");
        let mut b = TreeBuilder::new(&home, 1, dir());
        let cache = b.push(ROOT, OsStr::new(".cache"), dir()).unwrap();
        let pip = b.push(cache, OsStr::new("pip"), dir()).unwrap();
        let wheel = b.push(pip, OsStr::new("a.whl"), file(10_000)).unwrap();
        let notes = b.push(ROOT, OsStr::new("notes.txt"), file(100)).unwrap();
        let app = b.push(ROOT, OsStr::new("app"), dir()).unwrap();
        b.push(app, OsStr::new("data.db"), file(50)).unwrap();
        let tree = b.finish(Duration::ZERO);
        Fixture {
            tree,
            home,
            cache,
            pip,
            wheel,
            notes,
            app,
        }
    }

    fn review(f: &Fixture, ids: &[NodeId]) -> SieveReview {
        let advisor = Advisor::new(Platform::Linux, Some(&f.home));
        let guard = Guard::new(Some(f.home.clone()), [f.home.join("app")]);
        review_sieve(&f.tree, ids, &advisor, &guard)
    }

    fn status(r: &SieveReview, id: NodeId) -> SieveStatus {
        r.entries.iter().find(|e| e.id == id).unwrap().status
    }

    #[test]
    fn ready_items_are_summed_with_their_verdicts() {
        let f = fixture();
        let r = review(&f, &[f.pip, f.notes]);
        assert_eq!(status(&r, f.pip), SieveStatus::Ready);
        assert_eq!(status(&r, f.notes), SieveStatus::Ready);
        assert_eq!((r.safe, r.review, r.protected), (1, 1, 0));
        assert_eq!(r.logical_size, 10_100);
        assert_eq!(r.allocated_size, 12_288 + 4096);
        assert_eq!(r.file_count, 2);
        assert_eq!(r.trashable(), [f.pip, f.notes]);
        assert!(r.missing.is_empty());
    }

    #[test]
    fn items_inside_a_collected_folder_are_counted_once() {
        let f = fixture();
        // Collected child first, then its folder, then the folder above both.
        let r = review(&f, &[f.wheel, f.pip, f.cache]);
        assert_eq!(status(&r, f.cache), SieveStatus::Ready);
        assert_eq!(status(&r, f.wheel), SieveStatus::Covered(f.pip));
        assert_eq!(status(&r, f.pip), SieveStatus::Covered(f.cache));
        assert_eq!(r.trashable(), [f.cache]);
        assert_eq!(r.logical_size, 10_000);
        assert_eq!(r.file_count, 1);
        assert_eq!((r.safe, r.review), (1, 0));
    }

    #[test]
    fn protected_items_are_skipped_and_do_not_cover_their_contents() {
        let f = fixture();
        let data = f.tree.child_by_name(f.app, "data.db").unwrap();
        let r = review(&f, &[f.app, data, f.notes]);
        let entry = r.entries.iter().find(|e| e.id == f.app).unwrap();
        assert_eq!(entry.status, SieveStatus::Protected);
        assert_eq!(entry.advice.level, SafetyLevel::System);
        assert_eq!(entry.advice.protected, Some(Refusal::AppDirectory));
        // A refused folder moves nothing, so what is inside is judged on its own.
        assert_eq!(status(&r, data), SieveStatus::Protected);
        assert_eq!(r.protected, 2);
        assert_eq!(r.trashable(), [f.notes]);
        assert_eq!(r.logical_size, 100);
    }

    #[test]
    fn duplicates_are_removed_and_order_is_kept() {
        let f = fixture();
        let r = review(&f, &[f.notes, f.pip, f.notes]);
        let ids: Vec<_> = r.entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, [f.notes, f.pip]);
    }

    #[test]
    fn trashed_and_unknown_nodes_are_missing() {
        let mut f = fixture();
        f.tree.mark_trashed(f.pip).unwrap();
        let r = review(&f, &[f.pip, f.wheel, 9_999, f.notes]);
        assert_eq!(r.missing, [f.pip, f.wheel, 9_999]);
        assert_eq!(r.trashable(), [f.notes]);
    }

    #[test]
    fn scan_root_is_protected() {
        let f = fixture();
        let r = review(&f, &[ROOT]);
        assert_eq!(status(&r, ROOT), SieveStatus::Protected);
        assert_eq!(r.entries[0].advice.protected, Some(Refusal::ScanRoot));
        assert!(r.trashable().is_empty());
        assert_eq!(r.allocated_size, 0);
    }

    #[test]
    fn empty_basket_is_empty() {
        let f = fixture();
        assert_eq!(review(&f, &[]), SieveReview::default());
    }
}
