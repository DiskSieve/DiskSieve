//! Duplicate finder: files with identical content.
//!
//! Three passes, each cheaper than the next one and run only on what the previous left:
//!
//! 1. [`duplicate_candidates`] — files of the same exact logical size (from the tree alone,
//!    no disk access). Small files, hard links, online-only cloud files, links and anything
//!    trashed are left out; online-only files are never opened, since reading them would
//!    download them.
//! 2. Quick check — `blake3` of the first and the last 16 KiB of every candidate (one open,
//!    two reads). For files up to 32 KiB this already covers the whole content.
//! 3. Full `blake3` of the files still sharing size and quick hash, in parallel (`rayon`).
//!
//! Both disk passes can be cancelled and report progress. A file whose size changed since the
//! scan, or that cannot be read, is skipped and counted, never fatal.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rayon::prelude::*;

use crate::error::{CoreError, CoreResult};
use crate::scanner::CancelToken;
use crate::tree::{NodeFlags, NodeId, ScanTree, ROOT};

/// Size and content hash shared by the files of one group.
type GroupKey = (u64, [u8; 32]);
type Grouped = HashMap<GroupKey, Vec<Candidate>>;

/// Files smaller than this are not checked (the space they could free is negligible).
pub const MIN_DUPLICATE_SIZE: u64 = 4 * 1024;

/// Bytes hashed at each end of a file in the quick check.
const EDGE: u64 = 16 * 1024;
/// Read buffer of the full hash; cancellation is checked after every chunk.
const CHUNK: usize = 256 * 1024;

/// Files that are never compared: already counted elsewhere (hard links), not on this disk
/// (online-only cloud files), not regular files (links), unreadable or trashed.
const EXCLUDED: NodeFlags = NodeFlags::HARDLINK
    .union(NodeFlags::CLOUD_ONLINE)
    .union(NodeFlags::REPARSE)
    .union(NodeFlags::OTHER_VOLUME)
    .union(NodeFlags::ACCESS_DENIED)
    .union(NodeFlags::TRASHED);

/// A file that has at least one other file of the same size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: NodeId,
    /// Logical size recorded by the scan.
    pub size: u64,
    /// Modification time recorded by the scan (UNIX seconds).
    pub mtime: u32,
    pub path: PathBuf,
}

/// Files worth hashing, plus files left out because their path cannot be rebuilt exactly.
#[derive(Debug, Clone, Default)]
pub struct Candidates {
    /// Largest first.
    pub files: Vec<Candidate>,
    pub unresolved: u32,
}

/// Collects the files of `tree` that share their exact logical size with another file and are
/// at least `min_size` bytes. Reads the tree only — the disk is not touched.
pub fn duplicate_candidates(tree: &ScanTree, min_size: u64) -> Candidates {
    let mut by_size: HashMap<u64, Vec<NodeId>> = HashMap::new();
    let mut stack = vec![ROOT];
    while let Some(id) = stack.pop() {
        for child in tree.children(id) {
            let Some(node) = tree.get(child) else {
                continue;
            };
            if node.is_trashed() {
                continue; // with its whole subtree
            }
            if node.is_dir() {
                stack.push(child);
            } else if !node.flags.intersects(EXCLUDED) && node.logical_size >= min_size {
                by_size.entry(node.logical_size).or_default().push(child);
            }
        }
    }

    let mut out = Candidates::default();
    for (size, ids) in by_size {
        if ids.len() < 2 {
            continue;
        }
        for id in ids {
            match tree.resolve_path(id) {
                Ok(path) => out.files.push(Candidate {
                    id,
                    size,
                    mtime: tree.get(id).map_or(0, |n| n.newest_mtime),
                    path,
                }),
                Err(_) => out.unresolved += 1,
            }
        }
    }
    out.files
        .sort_unstable_by(|a, b| b.size.cmp(&a.size).then(a.id.cmp(&b.id)));
    out
}

/// Pass of the search that is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicatePhase {
    /// Hashing the first and last 16 KiB of every candidate.
    Quick,
    /// Hashing the whole content of the files that passed the quick check.
    Full,
}

/// Progress of [`find_duplicates`], sent at most once per `progress_interval` and once at the
/// start and the end of every pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateProgress {
    pub phase: DuplicatePhase,
    pub files_done: u32,
    pub files_total: u32,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Files with identical content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    /// Logical size of each copy.
    pub size: u64,
    /// `blake3` of the content.
    pub hash: [u8; 32],
    /// At least two; oldest first — the copy the UI suggests keeping.
    pub copies: Vec<Candidate>,
}

impl DuplicateGroup {
    /// Bytes freed by keeping only one copy.
    pub fn wasted(&self) -> u64 {
        self.size
            .saturating_mul(self.copies.len().saturating_sub(1) as u64)
    }

    /// Hash as 64 lowercase hex digits.
    pub fn hash_hex(&self) -> String {
        blake3::Hash::from_bytes(self.hash).to_hex().to_string()
    }
}

/// Result of [`find_duplicates`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DuplicateReport {
    /// Most wasted space first.
    pub groups: Vec<DuplicateGroup>,
    /// Candidates checked (same size as another file).
    pub files_checked: u32,
    /// Bytes read from disk by both passes.
    pub bytes_read: u64,
    /// Candidates that could not be read or changed since the scan.
    pub skipped: u32,
}

impl DuplicateReport {
    /// Bytes freed by keeping one copy of every group.
    pub fn wasted(&self) -> u64 {
        self.groups.iter().map(DuplicateGroup::wasted).sum()
    }
}

/// Hashes `candidates` (from [`duplicate_candidates`]) and returns the groups of identical
/// files. Fails only with [`CoreError::Cancelled`].
pub fn find_duplicates(
    candidates: Vec<Candidate>,
    cancel: &CancelToken,
    progress_interval: Duration,
    on_progress: impl Fn(DuplicateProgress) + Sync,
) -> CoreResult<DuplicateReport> {
    let files_checked = u32::try_from(candidates.len()).unwrap_or(u32::MAX);
    let skipped = AtomicU32::new(0);
    let bytes_read = AtomicU64::new(0);

    // Pass 2: quick hash of both ends.
    let quick_bytes = candidates.iter().map(|c| quick_len(c.size)).sum();
    let progress = Reporter::new(
        DuplicatePhase::Quick,
        candidates.len(),
        quick_bytes,
        progress_interval,
        &on_progress,
    );
    let quick: Vec<Option<[u8; 32]>> = candidates
        .par_iter()
        .map(|c| {
            if cancel.is_cancelled() {
                return None;
            }
            let hash = quick_hash(c);
            let read = quick_len(c.size);
            if hash.is_some() {
                bytes_read.fetch_add(read, Ordering::Relaxed);
            } else {
                skipped.fetch_add(1, Ordering::Relaxed);
            }
            progress.file_done(read);
            hash
        })
        .collect();
    if cancel.is_cancelled() {
        return Err(CoreError::Cancelled);
    }
    progress.finish();

    let mut by_quick: Grouped = HashMap::new();
    for (c, hash) in candidates.into_iter().zip(quick) {
        if let Some(hash) = hash {
            by_quick.entry((c.size, hash)).or_default().push(c);
        }
    }
    let mut finished: Vec<(GroupKey, Vec<Candidate>)> = Vec::new();
    let mut to_hash: Vec<Candidate> = Vec::new();
    for (key, files) in by_quick {
        if files.len() < 2 {
            continue;
        }
        if key.0 <= 2 * EDGE {
            finished.push((key, files)); // the quick hash already covered the whole file
        } else {
            to_hash.extend(files);
        }
    }

    // Pass 3: full hash of what is left.
    let progress = Reporter::new(
        DuplicatePhase::Full,
        to_hash.len(),
        to_hash.iter().map(|c| c.size).sum(),
        progress_interval,
        &on_progress,
    );
    let full: Vec<Option<[u8; 32]>> = to_hash
        .par_iter()
        .map(|c| {
            if cancel.is_cancelled() {
                return None;
            }
            let hash = full_hash(c, cancel, &progress, &bytes_read);
            if hash.is_none() && !cancel.is_cancelled() {
                skipped.fetch_add(1, Ordering::Relaxed);
            }
            progress.file_done(0);
            hash
        })
        .collect();
    if cancel.is_cancelled() {
        return Err(CoreError::Cancelled);
    }
    progress.finish();

    let mut by_full: Grouped = HashMap::new();
    for (c, hash) in to_hash.into_iter().zip(full) {
        if let Some(hash) = hash {
            by_full.entry((c.size, hash)).or_default().push(c);
        }
    }

    let mut groups: Vec<DuplicateGroup> = finished
        .into_iter()
        .chain(by_full)
        .filter(|(_, files)| files.len() >= 2)
        .map(|((size, hash), mut copies)| {
            copies.sort_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| a.path.cmp(&b.path)));
            DuplicateGroup { size, hash, copies }
        })
        .collect();
    groups.sort_by(|a, b| {
        b.wasted()
            .cmp(&a.wasted())
            .then(b.size.cmp(&a.size))
            .then_with(|| a.copies[0].path.cmp(&b.copies[0].path))
    });

    Ok(DuplicateReport {
        groups,
        files_checked,
        bytes_read: bytes_read.into_inner(),
        skipped: skipped.into_inner(),
    })
}

/// Bytes the quick check reads from a file of `size` bytes.
fn quick_len(size: u64) -> u64 {
    size.min(2 * EDGE)
}

/// Opens a candidate, refusing it when its size changed since the scan.
fn open_unchanged(c: &Candidate) -> Option<File> {
    let file = File::open(&c.path).ok()?;
    let meta = file.metadata().ok()?;
    (meta.is_file() && meta.len() == c.size).then_some(file)
}

/// `blake3` of the first and last 16 KiB (the whole file when it is at most 32 KiB).
fn quick_hash(c: &Candidate) -> Option<[u8; 32]> {
    let mut file = open_unchanged(c)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; EDGE as usize];
    let head = c.size.min(EDGE);
    read_into(&mut file, &mut buf[..head as usize])?;
    hasher.update(&buf[..head as usize]);
    if c.size > EDGE {
        let tail_start = c.size.saturating_sub(EDGE).max(EDGE);
        let tail = (c.size - tail_start) as usize;
        file.seek(SeekFrom::Start(tail_start)).ok()?;
        read_into(&mut file, &mut buf[..tail])?;
        hasher.update(&buf[..tail]);
    }
    Some(*hasher.finalize().as_bytes())
}

/// `blake3` of the whole content; `None` when unreadable, changed or cancelled.
fn full_hash(
    c: &Candidate,
    cancel: &CancelToken,
    progress: &Reporter<'_>,
    bytes_read: &AtomicU64,
) -> Option<[u8; 32]> {
    let mut file = open_unchanged(c)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if cancel.is_cancelled() {
            return None;
        }
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        hasher.update(&buf[..n]);
        total += n as u64;
        bytes_read.fetch_add(n as u64, Ordering::Relaxed);
        progress.bytes(n as u64);
    }
    // Grew or shrank while it was being read.
    (total == c.size).then(|| *hasher.finalize().as_bytes())
}

/// Fills `buf` completely; `None` on EOF or a read error.
fn read_into(file: &mut File, buf: &mut [u8]) -> Option<()> {
    file.read_exact(buf).ok()
}

/// Shared progress counters of one pass, throttled to `interval`.
struct Reporter<'a> {
    phase: DuplicatePhase,
    files_total: u32,
    bytes_total: u64,
    files_done: AtomicU32,
    bytes_done: AtomicU64,
    interval: Duration,
    last: Mutex<Instant>,
    sink: &'a (dyn Fn(DuplicateProgress) + Sync),
}

impl<'a> Reporter<'a> {
    fn new(
        phase: DuplicatePhase,
        files_total: usize,
        bytes_total: u64,
        interval: Duration,
        sink: &'a (dyn Fn(DuplicateProgress) + Sync),
    ) -> Self {
        let reporter = Self {
            phase,
            files_total: u32::try_from(files_total).unwrap_or(u32::MAX),
            bytes_total,
            files_done: AtomicU32::new(0),
            bytes_done: AtomicU64::new(0),
            interval,
            last: Mutex::new(Instant::now()),
            sink,
        };
        reporter.emit();
        reporter
    }

    fn file_done(&self, bytes: u64) {
        self.files_done.fetch_add(1, Ordering::Relaxed);
        self.bytes(bytes);
    }

    fn bytes(&self, bytes: u64) {
        self.bytes_done.fetch_add(bytes, Ordering::Relaxed);
        // Another thread reporting right now is as good as reporting here.
        let Ok(mut last) = self.last.try_lock() else {
            return;
        };
        if last.elapsed() >= self.interval {
            *last = Instant::now();
            drop(last);
            self.emit();
        }
    }

    fn finish(&self) {
        self.emit();
    }

    fn emit(&self) {
        (self.sink)(DuplicateProgress {
            phase: self.phase,
            files_done: self.files_done.load(Ordering::Relaxed),
            files_total: self.files_total,
            bytes_done: self.bytes_done.load(Ordering::Relaxed),
            bytes_total: self.bytes_total,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{scan, ScanOptions};
    use crate::tree::{EntryMeta, TreeBuilder};
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::time::SystemTime;
    use tempfile::TempDir;

    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn write(dir: &Path, rel: &str, data: &[u8]) -> PathBuf {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, data).unwrap();
        path
    }

    fn set_age(path: &Path, days: u64) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(days * 86_400))
            .unwrap();
    }

    fn scan_dir(dir: &Path) -> ScanTree {
        scan(dir, &ScanOptions::default(), &CancelToken::new(), |_| {}).unwrap()
    }

    fn run(tree: &ScanTree) -> DuplicateReport {
        let candidates = duplicate_candidates(tree, MIN_DUPLICATE_SIZE);
        find_duplicates(
            candidates.files,
            &CancelToken::new(),
            Duration::ZERO,
            |_| {},
        )
        .unwrap()
    }

    fn names(group: &DuplicateGroup) -> Vec<String> {
        group
            .copies
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn finds_identical_files_and_ignores_lookalikes() {
        let tmp = TempDir::new().unwrap();
        let big = pattern(100 * 1024, 1);
        let mut lookalike = big.clone();
        lookalike[50 * 1024] ^= 0xFF; // same size, same ends, different middle
        let new_copy = write(tmp.path(), "a/big.bin", &big);
        let old_copy = write(tmp.path(), "b/big-copy.bin", &big);
        write(tmp.path(), "c/lookalike.bin", &lookalike);
        write(tmp.path(), "a/small.dat", &pattern(10 * 1024, 7));
        write(tmp.path(), "b/small.dat", &pattern(10 * 1024, 7));
        write(tmp.path(), "a/tiny.txt", &pattern(1000, 3)); // below the minimum size
        write(tmp.path(), "b/tiny.txt", &pattern(1000, 3));
        write(tmp.path(), "a/unique.bin", &pattern(70 * 1024, 9));
        set_age(&new_copy, 1);
        set_age(&old_copy, 30);

        let report = run(&scan_dir(tmp.path()));

        assert_eq!(report.groups.len(), 2, "{report:#?}");
        let first = &report.groups[0];
        assert_eq!(first.size, 100 * 1024);
        assert_eq!(
            names(first),
            ["big-copy.bin", "big.bin"],
            "oldest copy first"
        );
        assert_eq!(first.wasted(), 100 * 1024);
        assert_eq!(first.hash_hex(), blake3::hash(&big).to_hex().to_string());
        assert_eq!(names(&report.groups[1]), ["small.dat", "small.dat"]);
        assert_eq!(report.wasted(), 110 * 1024);
        assert_eq!(report.files_checked, 5); // 3 × 100 KiB + 2 × 10 KiB
        assert_eq!(report.skipped, 0);
    }

    #[test]
    fn counts_every_extra_copy() {
        let tmp = TempDir::new().unwrap();
        let data = pattern(40 * 1024, 5);
        for name in ["one", "two", "three"] {
            write(tmp.path(), &format!("{name}/setup.exe"), &data);
        }
        let report = run(&scan_dir(tmp.path()));
        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].copies.len(), 3);
        assert_eq!(report.wasted(), 2 * 40 * 1024);
    }

    #[test]
    fn skips_files_changed_since_the_scan() {
        let tmp = TempDir::new().unwrap();
        let data = pattern(50 * 1024, 2);
        write(tmp.path(), "a.bin", &data);
        let b = write(tmp.path(), "b.bin", &data);
        let tree = scan_dir(tmp.path());
        let candidates = duplicate_candidates(&tree, MIN_DUPLICATE_SIZE);
        fs::write(&b, pattern(60 * 1024, 2)).unwrap();

        let report = find_duplicates(
            candidates.files,
            &CancelToken::new(),
            Duration::ZERO,
            |_| {},
        )
        .unwrap();
        assert!(report.groups.is_empty(), "{report:#?}");
        assert_eq!(report.skipped, 1);
    }

    #[test]
    fn cancellation_stops_the_search() {
        let tmp = TempDir::new().unwrap();
        let data = pattern(50 * 1024, 4);
        write(tmp.path(), "a.bin", &data);
        write(tmp.path(), "b.bin", &data);
        let tree = scan_dir(tmp.path());
        let cancel = CancelToken::new();
        cancel.cancel();
        let candidates = duplicate_candidates(&tree, MIN_DUPLICATE_SIZE);
        let result = find_duplicates(candidates.files, &cancel, Duration::ZERO, |_| {});
        assert!(matches!(result, Err(CoreError::Cancelled)));
    }

    #[test]
    fn reports_progress_of_both_passes() {
        let tmp = TempDir::new().unwrap();
        let data = pattern(80 * 1024, 6);
        write(tmp.path(), "a.bin", &data);
        write(tmp.path(), "b.bin", &data);
        let tree = scan_dir(tmp.path());
        let seen = Mutex::new(Vec::new());
        let candidates = duplicate_candidates(&tree, MIN_DUPLICATE_SIZE);
        find_duplicates(candidates.files, &CancelToken::new(), Duration::ZERO, |p| {
            seen.lock().unwrap().push(p)
        })
        .unwrap();
        let seen = seen.into_inner().unwrap();
        let last_of = |phase| *seen.iter().rev().find(|p| p.phase == phase).unwrap();
        let quick = last_of(DuplicatePhase::Quick);
        assert_eq!((quick.files_done, quick.files_total), (2, 2));
        assert_eq!(quick.bytes_done, quick.bytes_total);
        let full = last_of(DuplicatePhase::Full);
        assert_eq!((full.files_done, full.bytes_done), (2, 2 * 80 * 1024));
        assert_eq!(full.bytes_done, full.bytes_total);
    }

    #[test]
    fn candidates_leave_out_links_cloud_files_and_trashed_folders() {
        let dir = EntryMeta {
            flags: NodeFlags::DIR,
            ..EntryMeta::default()
        };
        let file = |flags: NodeFlags| EntryMeta {
            logical_size: 8192,
            allocated_size: 8192,
            mtime: 1_700_000_000,
            flags,
        };
        let mut b = TreeBuilder::new("/scan", 1, dir);
        let keep = b
            .push(ROOT, OsStr::new("plain.bin"), file(NodeFlags::empty()))
            .unwrap();
        let keep2 = b
            .push(ROOT, OsStr::new("plain2.bin"), file(NodeFlags::empty()))
            .unwrap();
        b.push(ROOT, OsStr::new("link.bin"), file(NodeFlags::HARDLINK))
            .unwrap();
        b.push(ROOT, OsStr::new("cloud.bin"), file(NodeFlags::CLOUD_ONLINE))
            .unwrap();
        b.push(ROOT, OsStr::new("junction"), file(NodeFlags::REPARSE))
            .unwrap();
        let old = b.push(ROOT, OsStr::new("old"), dir).unwrap();
        b.push(old, OsStr::new("gone.bin"), file(NodeFlags::empty()))
            .unwrap();
        let mut tree = b.finish(Duration::ZERO);
        tree.mark_trashed(old).unwrap();

        let candidates = duplicate_candidates(&tree, MIN_DUPLICATE_SIZE);
        let mut ids: Vec<NodeId> = candidates.files.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, [keep, keep2]);
        assert!(candidates.files[0].path.starts_with("/scan"));
    }
}
