//! # disksieve-core
//!
//! Pure Rust engine behind DiskSieve: filesystem scanning, the arena-based scan tree,
//! treemap viewport projection, dev-junk detection, duplicate hashing and snapshots.
//!
//! This crate has **no dependency on Tauri or any UI layer** — everything here is testable
//! with `cargo test -p disksieve-core`.

pub mod aggregator;
pub mod balance;
pub mod dev_junk;
pub mod error;
pub mod safety;
pub mod safety_advisor;
pub mod scanner;
pub mod search;
pub mod sieve;
pub mod tree;

pub use aggregator::{
    build_viewport, OtherBucket, SizeMetric, Viewport, ViewportItem, ViewportOptions,
    DEFAULT_MAX_ITEMS,
};
pub use balance::{is_volume_root, SpaceBalance};
pub use dev_junk::{contains_junk, detect_junk, junk_projects, JunkKind, JunkProject};
pub use error::{CoreError, CoreResult};
pub use safety::{
    move_to_system_trash, restore_from_system_trash, restore_nodes, trash_nodes, verify_unchanged,
    Freed, Guard, Refusal, TrashError, TrashOutcome, RESTORE_SUPPORTED,
};
pub use safety_advisor::{
    Advice, Advisor, Alternative, Platform, Rule, SafetyLevel, DEV_JUNK_RULE, RULES,
};
pub use scanner::{scan, CancelToken, ScanOptions, ScanProgress};
pub use search::{search_nodes, SearchResult, DEFAULT_SEARCH_LIMIT};
pub use sieve::{review_sieve, SieveEntry, SieveReview, SieveStatus};
pub use tree::{
    EntryMeta, Node, NodeFlags, NodeId, ScanStats, ScanTree, TreeBuilder, NO_NODE, ROOT,
};

/// Version of the core engine (taken from the workspace manifest).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver() {
        assert_eq!(VERSION.split('.').count(), 3, "{VERSION}");
    }
}
