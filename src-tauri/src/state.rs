//! Shared application state: the last completed scan tree and the scan in progress.
//!
//! Only one scan runs at a time. Starting a new scan cancels the previous one; a result is
//! accepted only if it belongs to the newest scan (`generation`), so a slow, superseded scan
//! can never overwrite a newer tree.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use disksieve_core::{CancelToken, Delta, DuplicateReport, ScanTree, Snapshot};

/// Handle of the scan currently running.
#[derive(Debug, Clone)]
pub struct ScanHandle {
    pub generation: u64,
    pub cancel: CancelToken,
}

/// Previous snapshot of the scanned root and the Delta Mode overlay computed from it.
#[derive(Debug)]
pub struct Baseline {
    pub snapshot: Snapshot,
    pub delta: Delta,
}

impl Baseline {
    pub fn new(tree: &ScanTree, snapshot: Snapshot) -> Self {
        let delta = Delta::compute(tree, &snapshot);
        Self { snapshot, delta }
    }

    /// Recomputes the overlay after the tree changed (trash / restore).
    pub fn refresh(&mut self, tree: &ScanTree) {
        self.delta = Delta::compute(tree, &self.snapshot);
    }
}

/// Handle of the duplicate search currently running.
#[derive(Debug, Clone)]
pub struct DuplicateSearch {
    pub id: u64,
    pub cancel: CancelToken,
}

/// Result of the last duplicate search, for the tree of `generation`.
#[derive(Debug)]
pub struct DuplicateResults {
    pub generation: u64,
    pub report: DuplicateReport,
    pub duration: Duration,
}

/// State managed by Tauri (`app.manage(AppState::default())`).
#[derive(Debug, Default)]
pub struct AppState {
    tree: RwLock<Option<ScanTree>>,
    /// Belongs to the tree above; `None` when the root was never scanned before.
    baseline: RwLock<Option<Baseline>>,
    scan: Mutex<Option<ScanHandle>>,
    last_generation: AtomicU64,
    /// Last duplicate search; belongs to the tree when its generation matches.
    duplicates: RwLock<Option<DuplicateResults>>,
    search: Mutex<Option<DuplicateSearch>>,
    last_search: AtomicU64,
}

impl AppState {
    /// Registers a new scan, cancelling the one in progress (if any).
    pub fn begin_scan(&self) -> ScanHandle {
        let generation = self.last_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let handle = ScanHandle {
            generation,
            cancel: CancelToken::new(),
        };
        if let Some(previous) = self.scan_slot().replace(handle.clone()) {
            previous.cancel.cancel();
        }
        handle
    }

    /// Ends scan `generation`. Stores `tree` (with its Delta baseline) and returns `true` only
    /// if this is still the newest scan; a superseded scan's result is dropped.
    pub fn finish_scan(
        &self,
        generation: u64,
        tree: Option<ScanTree>,
        baseline: Option<Baseline>,
    ) -> bool {
        let mut slot = self.scan_slot();
        if slot.as_ref().map(|h| h.generation) != Some(generation) {
            return false;
        }
        *slot = None;
        match tree {
            Some(tree) => {
                // Lock order: scan slot, then tree, then baseline, then duplicates — the same
                // everywhere.
                let mut current = self.tree_mut();
                *current = Some(tree);
                *self.baseline_mut() = baseline;
                *self.duplicates_mut() = None;
                true
            }
            None => false,
        }
    }

    /// Cancels the scan in progress. Returns `false` when no scan is running.
    pub fn cancel_scan(&self) -> bool {
        match self.scan_slot().as_ref() {
            Some(handle) => {
                handle.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Read access to the last completed scan tree.
    pub fn tree(&self) -> RwLockReadGuard<'_, Option<ScanTree>> {
        self.tree.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Write access to the last completed scan tree (trash / restore update it in place).
    pub fn tree_mut(&self) -> RwLockWriteGuard<'_, Option<ScanTree>> {
        self.tree.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Read access to the Delta baseline of the current tree.
    pub fn baseline(&self) -> RwLockReadGuard<'_, Option<Baseline>> {
        self.baseline.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Write access to the Delta baseline (refreshed after trash / restore).
    pub fn baseline_mut(&self) -> RwLockWriteGuard<'_, Option<Baseline>> {
        self.baseline
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a new duplicate search, cancelling the one in progress (if any).
    pub fn begin_duplicates(&self) -> DuplicateSearch {
        let search = DuplicateSearch {
            id: self.last_search.fetch_add(1, Ordering::Relaxed) + 1,
            cancel: CancelToken::new(),
        };
        if let Some(previous) = self.search_slot().replace(search.clone()) {
            previous.cancel.cancel();
        }
        search
    }

    /// Ends duplicate search `id`, storing `results` only if it is still the newest search.
    pub fn finish_duplicates(&self, id: u64, results: Option<DuplicateResults>) -> bool {
        let mut slot = self.search_slot();
        if slot.as_ref().map(|s| s.id) != Some(id) {
            return false;
        }
        *slot = None;
        match results {
            Some(results) => {
                *self.duplicates_mut() = Some(results);
                true
            }
            None => false,
        }
    }

    /// Cancels the duplicate search in progress. Returns `false` when none is running.
    pub fn cancel_duplicates(&self) -> bool {
        match self.search_slot().as_ref() {
            Some(search) => {
                search.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Result of the last duplicate search (check its `generation` against the tree).
    pub fn duplicates(&self) -> RwLockReadGuard<'_, Option<DuplicateResults>> {
        self.duplicates
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn duplicates_mut(&self) -> RwLockWriteGuard<'_, Option<DuplicateResults>> {
        self.duplicates
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn search_slot(&self) -> MutexGuard<'_, Option<DuplicateSearch>> {
        self.search.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn scan_slot(&self) -> MutexGuard<'_, Option<ScanHandle>> {
        // A panic while holding the lock cannot leave the handle half-written, so the data
        // is still consistent and the poison flag can be ignored.
        self.scan.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use disksieve_core::{scan, ScanOptions};

    fn scan_dir(dir: &std::path::Path, generation: u64) -> ScanTree {
        let opts = ScanOptions {
            generation,
            ..ScanOptions::default()
        };
        scan(dir, &opts, &CancelToken::new(), |_| {}).unwrap()
    }

    #[test]
    fn only_the_newest_duplicate_search_is_stored() {
        let state = AppState::default();
        let old = state.begin_duplicates();
        let new = state.begin_duplicates();
        assert!(
            old.cancel.is_cancelled(),
            "a new search cancels the old one"
        );
        let results = |generation| DuplicateResults {
            generation,
            report: DuplicateReport::default(),
            duration: Duration::ZERO,
        };
        assert!(!state.finish_duplicates(old.id, Some(results(1))));
        assert!(state.duplicates().is_none());
        assert!(state.cancel_duplicates());
        assert!(state.finish_duplicates(new.id, Some(results(2))));
        assert_eq!(state.duplicates().as_ref().map(|r| r.generation), Some(2));
        assert!(!state.cancel_duplicates(), "nothing is running any more");
    }

    #[test]
    fn generations_increase_and_new_scan_cancels_previous() {
        let state = AppState::default();
        let first = state.begin_scan();
        let second = state.begin_scan();
        assert_eq!(first.generation, 1);
        assert_eq!(second.generation, 2);
        assert!(first.cancel.is_cancelled());
        assert!(!second.cancel.is_cancelled());
    }

    #[test]
    fn newest_scan_result_is_stored() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), b"hello").unwrap();
        let state = AppState::default();
        let handle = state.begin_scan();
        assert!(state.finish_scan(
            handle.generation,
            Some(scan_dir(tmp.path(), handle.generation)),
            None
        ));
        let tree = state.tree();
        assert_eq!(tree.as_ref().unwrap().generation(), handle.generation);
        drop(tree);
        assert!(!state.cancel_scan(), "nothing running after finish");
    }

    #[test]
    fn superseded_scan_result_is_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let old = state.begin_scan();
        let new = state.begin_scan();
        assert!(!state.finish_scan(
            old.generation,
            Some(scan_dir(tmp.path(), old.generation)),
            None
        ));
        assert!(state.tree().is_none());
        assert!(state.cancel_scan(), "the newer scan is still running");
        assert!(state.finish_scan(
            new.generation,
            Some(scan_dir(tmp.path(), new.generation)),
            None
        ));
        assert_eq!(state.tree().as_ref().unwrap().generation(), new.generation);
    }

    #[test]
    fn cancelled_scan_keeps_previous_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let first = state.begin_scan();
        assert!(state.finish_scan(
            first.generation,
            Some(scan_dir(tmp.path(), first.generation)),
            None
        ));

        let second = state.begin_scan();
        assert!(state.cancel_scan());
        assert!(second.cancel.is_cancelled());
        assert!(!state.finish_scan(second.generation, None, None));
        assert_eq!(
            state.tree().as_ref().unwrap().generation(),
            first.generation
        );
        assert!(!state.cancel_scan());
    }

    #[test]
    fn baseline_follows_the_stored_tree() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let state = AppState::default();
        let first = state.begin_scan();
        let tree = scan_dir(tmp.path(), first.generation);
        let baseline = Baseline::new(&tree, Snapshot::capture(&tree));
        assert!(state.finish_scan(first.generation, Some(tree), Some(baseline)));
        assert_eq!(state.baseline().as_ref().unwrap().delta.bytes(0), Some(0));

        let old = state.begin_scan();
        let new = state.begin_scan();
        let tree = scan_dir(tmp.path(), old.generation);
        let stale = Baseline::new(&tree, Snapshot::capture(&tree));
        assert!(!state.finish_scan(old.generation, Some(tree), Some(stale)));
        assert!(
            state.baseline().is_some(),
            "a superseded scan keeps the baseline"
        );

        let tree = scan_dir(tmp.path(), new.generation);
        assert!(state.finish_scan(new.generation, Some(tree), None));
        assert!(
            state.baseline().is_none(),
            "a root without a snapshot has no baseline"
        );
    }
}
