//! Name search over the scanned tree (`Ctrl+K`).
//!
//! Works only on the arena's name buffer — the disk is never touched, so a search over a
//! million entries takes milliseconds and works for cloud-only files too.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::aggregator::SizeMetric;
use crate::tree::{NodeId, ScanTree, ROOT};

/// Default number of hits returned to the UI.
pub const DEFAULT_SEARCH_LIMIT: usize = 50;

/// Result of [`search_nodes`]: the largest matches, largest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// At most `limit` matching nodes, ordered by the chosen size metric (descending),
    /// ties by name.
    pub hits: Vec<NodeId>,
    /// How many nodes matched in total (may be larger than `hits.len()`).
    pub total_matches: usize,
}

/// A query split into lower-cased terms; a name matches when it contains every term.
struct Query {
    terms: Vec<String>,
    ascii: bool,
}

impl Query {
    fn parse(query: &str) -> Self {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let ascii = terms.iter().all(|t| t.is_ascii());
        Self { terms, ascii }
    }

    fn matches(&self, name: &str, lowered: &mut String) -> bool {
        if self.ascii && name.is_ascii() {
            return self
                .terms
                .iter()
                .all(|t| contains_ignore_ascii_case(name.as_bytes(), t.as_bytes()));
        }
        lowered.clear();
        lowered.extend(name.chars().flat_map(char::to_lowercase));
        self.terms.iter().all(|t| lowered.contains(t.as_str()))
    }
}

/// `haystack` contains `needle` (already lower-case ASCII), ignoring ASCII case.
fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack.windows(needle.len()).any(|w| {
            w.iter()
                .zip(needle)
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
        })
}

/// Finds files and folders whose name contains every whitespace-separated term of
/// `query` (case-insensitive) and returns the `limit` largest by `metric`.
///
/// The root itself and everything moved to Trash (including the contents of trashed
/// folders) is skipped. An empty query matches nothing.
pub fn search_nodes(
    tree: &ScanTree,
    query: &str,
    metric: SizeMetric,
    limit: usize,
) -> SearchResult {
    let query = Query::parse(query);
    let mut result = SearchResult {
        hits: Vec::new(),
        total_matches: 0,
    };
    if query.terms.is_empty() {
        return result;
    }

    // Min-heap of the best `limit` hits: (size, Reverse(name)) so the smallest size — and
    // for equal sizes the alphabetically last name — is evicted first.
    let mut best: BinaryHeap<Reverse<(u64, Reverse<&str>, NodeId)>> =
        BinaryHeap::with_capacity(limit + 1);
    let mut lowered = String::new();
    let mut stack: Vec<NodeId> = tree.children(ROOT).collect();

    while let Some(id) = stack.pop() {
        let Some(node) = tree.get(id) else { continue };
        if node.is_trashed() {
            continue;
        }
        if node.is_dir() {
            stack.extend(tree.children(id));
        }
        let name = tree.name(id);
        if !query.matches(name, &mut lowered) {
            continue;
        }
        result.total_matches += 1;
        if limit == 0 {
            continue;
        }
        best.push(Reverse((metric.value(node), Reverse(name), id)));
        if best.len() > limit {
            best.pop();
        }
    }

    // `into_sorted_vec` on `Reverse` items yields largest first.
    result.hits = best
        .into_sorted_vec()
        .into_iter()
        .map(|Reverse((_, _, id))| id)
        .collect();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{EntryMeta, NodeFlags, TreeBuilder};
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::time::Duration;

    fn file(logical: u64, allocated: u64) -> EntryMeta {
        EntryMeta {
            logical_size: logical,
            allocated_size: allocated,
            mtime: 1_000,
            flags: NodeFlags::empty(),
        }
    }

    fn dir() -> EntryMeta {
        EntryMeta {
            flags: NodeFlags::DIR,
            ..EntryMeta::default()
        }
    }

    /// root/
    /// ├── Videos/
    /// │   ├── holiday.MP4    5 000 / 8 192
    /// │   └── Zażółć.mov     9 000 / 4 096
    /// ├── projects/
    /// │   └── video-tool/
    /// │       └── main.rs      100 / 4 096
    /// └── notes.txt            10 / 4 096
    fn sample() -> (ScanTree, [NodeId; 6]) {
        let root = PathBuf::from(if cfg!(windows) { r"C:\scan" } else { "/scan" });
        let mut b = TreeBuilder::new(root, 1, dir());
        let mut push = |parent, name: &str, meta| b.push(parent, OsStr::new(name), meta).unwrap();
        let videos = push(ROOT, "Videos", dir());
        let holiday = push(videos, "holiday.MP4", file(5_000, 8_192));
        let zazolc = push(videos, "Zażółć.mov", file(9_000, 4_096));
        let projects = push(ROOT, "projects", dir());
        let tool = push(projects, "video-tool", dir());
        let main = push(tool, "main.rs", file(100, 4_096));
        push(ROOT, "notes.txt", file(10, 4_096));
        let tree = b.finish(Duration::ZERO);
        (tree, [videos, holiday, zazolc, tool, main, projects])
    }

    fn search(tree: &ScanTree, q: &str) -> SearchResult {
        search_nodes(tree, q, SizeMetric::Allocated, DEFAULT_SEARCH_LIMIT)
    }

    #[test]
    fn matches_files_and_folders_case_insensitively_largest_first() {
        let (tree, [videos, .., tool, _, _]) = sample();
        let r = search(&tree, "VIDEO");
        assert_eq!(r.hits, vec![videos, tool]);
        assert_eq!(r.total_matches, 2);
    }

    #[test]
    fn every_term_must_match() {
        let (tree, [_, holiday, ..]) = sample();
        assert_eq!(search(&tree, "hol mp4").hits, vec![holiday]);
        assert!(search(&tree, "hol mov").hits.is_empty());
    }

    #[test]
    fn non_ascii_names_match_with_unicode_case_folding() {
        let (tree, [_, _, zazolc, ..]) = sample();
        assert_eq!(search(&tree, "ŻÓŁĆ").hits, vec![zazolc]);
        assert_eq!(search(&tree, "zaż").hits, vec![zazolc]);
    }

    #[test]
    fn metric_changes_the_order() {
        let (tree, [videos, holiday, zazolc, ..]) = sample();
        let by_disk = search_nodes(&tree, "o", SizeMetric::Allocated, 2);
        let by_logical = search_nodes(&tree, "o", SizeMetric::Logical, 2);
        assert_eq!(by_disk.hits, vec![videos, holiday]);
        assert_eq!(by_logical.hits, vec![videos, zazolc]);
    }

    #[test]
    fn limit_keeps_the_largest_but_counts_all_matches() {
        let (tree, [videos, ..]) = sample();
        let r = search_nodes(&tree, "o", SizeMetric::Allocated, 1);
        assert_eq!(r.hits, vec![videos]);
        assert!(r.total_matches > 1);
        let none = search_nodes(&tree, "o", SizeMetric::Allocated, 0);
        assert!(none.hits.is_empty());
        assert_eq!(none.total_matches, r.total_matches);
    }

    #[test]
    fn empty_query_and_root_never_match() {
        let (tree, _) = sample();
        assert_eq!(search(&tree, "   ").total_matches, 0);
        assert!(!search(&tree, "scan").hits.contains(&ROOT));
    }

    #[test]
    fn trashed_nodes_and_their_contents_are_skipped() {
        let (mut tree, [_, _, _, tool, main, projects]) = sample();
        tree.mark_trashed(tool).unwrap();
        assert!(search(&tree, "main").hits.is_empty());
        assert!(search(&tree, "video-tool").hits.is_empty());
        assert_eq!(search(&tree, "projects").hits, vec![projects]);
        tree.restore_trashed(tool).unwrap();
        assert_eq!(search(&tree, "main").hits, vec![main]);
    }

    #[test]
    fn ties_are_ordered_by_name() {
        let root = PathBuf::from(if cfg!(windows) { r"C:\scan" } else { "/scan" });
        let mut b = TreeBuilder::new(root, 1, dir());
        let c = b.push(ROOT, OsStr::new("c.log"), file(1, 4_096)).unwrap();
        let a = b.push(ROOT, OsStr::new("a.log"), file(1, 4_096)).unwrap();
        let bb = b.push(ROOT, OsStr::new("b.log"), file(1, 4_096)).unwrap();
        let tree = b.finish(Duration::ZERO);
        assert_eq!(
            search_nodes(&tree, "log", SizeMetric::Allocated, 2).hits,
            vec![a, bb]
        );
        assert_eq!(search(&tree, "log").hits, vec![a, bb, c]);
    }
}
