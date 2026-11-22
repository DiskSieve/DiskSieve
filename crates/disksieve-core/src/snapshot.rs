//! Scan snapshots and the Delta Mode overlay (spec 2.7).
//!
//! A snapshot keeps only folders and files of at least [`MIN_FILE_SIZE`]: a 64-bit hash of the
//! full path, the hash of the parent's path, the name and the size on disk. That keeps it small
//! even for millions of nodes. The next scan of the same root compares against it ([`Delta`])
//! to show what grew, shrank, appeared or disappeared — removed items keep their name.
//!
//! File layout: magic `DSNP`, the format version (`u16`, little endian), then a `zstd` frame
//! (with checksum) holding the `postcard`-encoded body. Reading never panics: a file from
//! another format version is [`SnapshotError::Incompatible`], anything unreadable is
//! [`SnapshotError::Corrupt`].

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tree::{NodeId, ScanTree};

/// Current snapshot format; bump it whenever the body layout changes.
/// v2 (step 6.2): parent hash, name and kind of every entry.
pub const SNAPSHOT_FORMAT_VERSION: u16 = 2;
/// Smallest file kept in a snapshot (folders are always kept).
pub const MIN_FILE_SIZE: u64 = 1024 * 1024;
/// Extension of snapshot files.
pub const SNAPSHOT_EXTENSION: &str = "dsnap";

const MAGIC: &[u8; 4] = b"DSNP";
/// Upper bound for a decompressed body, so a damaged file cannot exhaust memory.
const MAX_BODY_BYTES: u64 = 1 << 30;
const ZSTD_LEVEL: i32 = 3;
/// Parent hash of the root entry.
const NO_PARENT: u64 = 0;

/// Why a snapshot could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// Written by another DiskSieve version with a different format; it is ignored.
    #[error("snapshot format {found} is not supported (expected {SNAPSHOT_FORMAT_VERSION})")]
    Incompatible { found: u16 },
    /// Not a snapshot, truncated or damaged.
    #[error("snapshot file is damaged: {0}")]
    Corrupt(String),
    #[error("snapshot I/O error at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// One stored folder or large file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    /// Hash of the full path (see [`path_hashes`]).
    pub hash: u64,
    /// Hash of the parent's full path (0 for the scan root).
    pub parent: u64,
    /// Size on disk this item added to its parent.
    pub allocated: u64,
    pub is_dir: bool,
    /// File or folder name (the full root path for the scan root).
    pub name: String,
}

#[derive(Serialize, Deserialize)]
struct Body {
    root: String,
    taken_at: u32,
    /// Sorted by hash, hashes unique.
    entries: Vec<SnapshotEntry>,
}

/// Sizes of one scan, keyed by path hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    root: String,
    taken_at: u32,
    entries: Vec<SnapshotEntry>,
}

impl Snapshot {
    /// Captures folders and files ≥ [`MIN_FILE_SIZE`] of `tree`; items moved to Trash (and
    /// everything inside them) are left out. Sizes are what each item adds to its parent, so
    /// hard links count once.
    pub fn capture(tree: &ScanTree) -> Self {
        let hashes = path_hashes(tree);
        let alive = alive_nodes(tree);
        let mut entries: Vec<SnapshotEntry> = (0..tree.len())
            .filter(|&i| alive[i])
            .filter_map(|i| {
                let id = i as NodeId;
                let node = tree.get(id)?;
                let (_, allocated, _) = node.contribution();
                let root = i == 0;
                (node.is_dir() || allocated >= MIN_FILE_SIZE).then(|| SnapshotEntry {
                    hash: hashes[i],
                    parent: if root {
                        NO_PARENT
                    } else {
                        hashes[node.parent as usize]
                    },
                    allocated,
                    is_dir: node.is_dir(),
                    name: if root {
                        tree.root_path().to_string_lossy().into_owned()
                    } else {
                        tree.name(id).to_owned()
                    },
                })
            })
            .collect();
        entries.sort_unstable_by_key(|e| e.hash);
        // Two paths with the same 64-bit hash are vanishingly rare; keep the first.
        entries.dedup_by_key(|e| e.hash);
        Self {
            root: tree.root_path().to_string_lossy().into_owned(),
            taken_at: tree.stats().started_at,
            entries,
        }
    }

    /// Scan root the snapshot was taken of.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// When the scan started, seconds since the UNIX epoch.
    pub fn taken_at(&self) -> u32 {
        self.taken_at
    }

    /// Stored items, sorted by path hash.
    pub fn entries(&self) -> &[SnapshotEntry] {
        &self.entries
    }

    /// Number of stored items.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The stored item with this path hash.
    pub fn get(&self, path_hash: u64) -> Option<&SnapshotEntry> {
        self.entries
            .binary_search_by_key(&path_hash, |e| e.hash)
            .ok()
            .map(|i| &self.entries[i])
    }

    /// Hash under which folder `root` is stored: the snapshot's own root, or a folder inside
    /// it (found by name, so a drive snapshot also serves scans of its folders). `None` when
    /// `root` is not in this snapshot.
    pub fn hash_of(&self, root: &Path) -> Option<u64> {
        let own_root = Path::new(&self.root);
        let mut hash = fnv1a(FNV_OFFSET, own_root.as_os_str().as_encoded_bytes());
        self.get(hash)?;
        for part in root.strip_prefix(own_root).ok()?.components() {
            let Component::Normal(name) = part else {
                return None;
            };
            let name = name.to_string_lossy();
            hash = self
                .entries
                .iter()
                .find(|e| e.parent == hash && e.is_dir && same_name(&e.name, &name))?
                .hash;
        }
        Some(hash)
    }

    /// Size on disk of the item with this path hash, if it was stored.
    pub fn allocated(&self, path_hash: u64) -> Option<u64> {
        self.get(path_hash).map(|e| e.allocated)
    }

    /// Writes the snapshot in the versioned file format.
    pub fn write_to(&self, mut out: impl Write) -> io::Result<()> {
        out.write_all(MAGIC)?;
        out.write_all(&SNAPSHOT_FORMAT_VERSION.to_le_bytes())?;
        let body = Body {
            root: self.root.clone(),
            taken_at: self.taken_at,
            entries: self.entries.clone(),
        };
        let bytes = postcard::to_stdvec(&body).map_err(io::Error::other)?;
        let mut encoder = zstd::Encoder::new(out, ZSTD_LEVEL)?;
        // Frame checksum: a flipped byte is reported as damage instead of wrong sizes.
        encoder.include_checksum(true)?;
        encoder.write_all(&bytes)?;
        encoder.finish()?.flush()
    }

    /// Reads a snapshot written by [`Snapshot::write_to`].
    pub fn read_from(mut input: impl Read) -> Result<Self, SnapshotError> {
        let corrupt = |what: &str| SnapshotError::Corrupt(what.to_owned());
        let mut header = [0u8; 6];
        input
            .read_exact(&mut header)
            .map_err(|_| corrupt("file is too short"))?;
        if &header[..4] != MAGIC {
            return Err(corrupt("not a DiskSieve snapshot"));
        }
        let found = u16::from_le_bytes([header[4], header[5]]);
        if found != SNAPSHOT_FORMAT_VERSION {
            return Err(SnapshotError::Incompatible { found });
        }
        let mut bytes = Vec::new();
        zstd::Decoder::new(input)
            .and_then(|d| d.take(MAX_BODY_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
        if bytes.len() as u64 > MAX_BODY_BYTES {
            return Err(corrupt("body is too large"));
        }
        let body: Body =
            postcard::from_bytes(&bytes).map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
        if !body.entries.windows(2).all(|w| w[0].hash < w[1].hash) {
            return Err(corrupt("entries are not sorted"));
        }
        Ok(Self {
            root: body.root,
            taken_at: body.taken_at,
            entries: body.entries,
        })
    }

    /// Saves to `path` atomically (temporary file, then rename), creating parent folders.
    pub fn save(&self, path: &Path) -> Result<(), SnapshotError> {
        let io_err = |source| SnapshotError::Io {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io_err)?;
        }
        let tmp = path.with_extension("tmp");
        let written = fs::File::create(&tmp).and_then(|file| {
            let mut out = io::BufWriter::new(file);
            self.write_to(&mut out)?;
            out.into_inner().map_err(|e| e.into_error())?.sync_all()
        });
        if let Err(e) = written.and_then(|()| fs::rename(&tmp, path)) {
            // Our own temporary file, never user data.
            let _ = fs::remove_file(&tmp);
            return Err(io_err(e));
        }
        Ok(())
    }

    /// Loads `path`; `Ok(None)` when there is no snapshot yet.
    pub fn load(path: &Path) -> Result<Option<Self>, SnapshotError> {
        match fs::File::open(path) {
            Ok(file) => Self::read_from(io::BufReader::new(file)).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(SnapshotError::Io {
                path: path.to_owned(),
                source,
            }),
        }
    }
}

/// File (inside a snapshot folder) holding the latest snapshot of scan root `root`.
pub fn snapshot_path(dir: &Path, root: &Path) -> PathBuf {
    let hash = fnv1a(FNV_OFFSET, root.as_os_str().as_encoded_bytes());
    dir.join(format!("{hash:016x}.{SNAPSHOT_EXTENSION}"))
}

/// Newest snapshot in `dir` that covers `root`: a snapshot of `root` itself or of a folder
/// above it (e.g. the whole drive). A snapshot of another root (hash collision of the file
/// name) counts as none. Errors are reported only when no snapshot was found; an unreadable
/// snapshot of a folder above `root` is skipped silently (it has its own warning when that
/// folder is scanned).
pub fn load_previous(dir: &Path, root: &Path) -> Result<Option<Snapshot>, SnapshotError> {
    let mut best: Option<Snapshot> = None;
    let mut error = None;
    for (i, candidate) in root.ancestors().enumerate() {
        match Snapshot::load(&snapshot_path(dir, candidate)) {
            Ok(Some(s)) if s.root == candidate.to_string_lossy() && s.hash_of(root).is_some() => {
                if best.as_ref().is_none_or(|b| s.taken_at > b.taken_at) {
                    best = Some(s);
                }
            }
            Ok(_) => {}
            Err(e) if i == 0 => error = Some(e),
            Err(_) => {}
        }
    }
    match (best, error) {
        (None, Some(e)) => Err(e),
        (best, _) => Ok(best),
    }
}

/// 64-bit hash of every node's full path, indexed by `NodeId`. Stable across scans and app
/// versions (FNV-1a), so it can key a snapshot.
pub fn path_hashes(tree: &ScanTree) -> Vec<u64> {
    let root = fnv1a(FNV_OFFSET, tree.root_path().as_os_str().as_encoded_bytes());
    path_hashes_from(tree, root)
}

/// Like [`path_hashes`], with the scan root hashed as `root` — its hash in a snapshot of a
/// folder above it.
fn path_hashes_from(tree: &ScanTree, root: u64) -> Vec<u64> {
    let mut hashes = Vec::with_capacity(tree.len());
    hashes.push(root);
    // Children always come after their parent in the arena.
    for i in 1..tree.len() {
        let parent = tree.get(i as u32).map_or(0, |n| n.parent as usize);
        let base = fnv1a(hashes[parent], b"/");
        hashes.push(fnv1a(base, tree.name(i as u32).as_bytes()));
    }
    hashes
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// File names compare case-insensitively on Windows (its file systems ignore case).
fn same_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a == b || a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// `true` for nodes that are neither moved to Trash nor inside a trashed folder.
fn alive_nodes(tree: &ScanTree) -> Vec<bool> {
    let mut alive = Vec::with_capacity(tree.len());
    for i in 0..tree.len() {
        let node = tree.get(i as NodeId);
        let parent_alive = i == 0 || node.is_some_and(|n| alive[n.parent as usize]);
        alive.push(parent_alive && node.is_some_and(|n| !n.is_trashed()));
    }
    alive
}

/// An item of the previous scan that is gone now (deleted, moved, or moved to Trash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedEntry {
    /// Closest folder of the current scan it was in.
    pub parent: NodeId,
    pub name: String,
    /// Its size on disk in the previous scan.
    pub allocated: u64,
    pub is_dir: bool,
}

/// Delta Mode overlay: change of every node since a snapshot of the same root, kept next to
/// the tree (never in the node itself).
///
/// * Stored item: current size − size in the snapshot.
/// * Folder or file ≥ [`MIN_FILE_SIZE`] missing from the snapshot: new, its whole size.
/// * Smaller file missing from the snapshot: 0 — small files are not stored, so their
///   changes only show in the folder sums.
/// * Stored item missing from the tree: a [`RemovedEntry`] in its folder (only the topmost
///   removed item of a branch; its size includes what was inside).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delta {
    taken_at: u32,
    bytes: Vec<i64>,
    /// Size of each node in the snapshot; `None` when it was not stored (new, a small file,
    /// or moved to Trash).
    before: Vec<Option<u64>>,
    /// Sorted by parent, then largest first.
    removed: Vec<RemovedEntry>,
}

impl Delta {
    /// Compares `tree` with `snapshot`, taken of the same root or of a folder above it (see
    /// [`load_previous`]). Against a snapshot that does not contain the root, everything
    /// counts as new.
    pub fn compute(tree: &ScanTree, snapshot: &Snapshot) -> Self {
        let hashes = match snapshot.hash_of(tree.root_path()) {
            Some(root) => path_hashes_from(tree, root),
            None => path_hashes(tree),
        };
        let alive = alive_nodes(tree);
        let signed = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        let before: Vec<Option<u64>> = (0..tree.len())
            .map(|i| alive[i].then(|| snapshot.allocated(hashes[i])).flatten())
            .collect();
        let bytes = (0..tree.len())
            .map(|i| {
                let Some(node) = tree.get(i as NodeId).filter(|_| alive[i]) else {
                    return 0;
                };
                let (_, allocated, _) = node.contribution();
                match before[i] {
                    Some(before) => signed(allocated).saturating_sub(signed(before)),
                    None if node.is_dir() || allocated >= MIN_FILE_SIZE => signed(allocated),
                    None => 0,
                }
            })
            .collect();

        let mut current: Vec<(u64, NodeId)> = (0..tree.len())
            .filter(|&i| alive[i])
            .map(|i| (hashes[i], i as NodeId))
            .collect();
        current.sort_unstable_by_key(|&(hash, _)| hash);
        let find = |hash: u64| {
            current
                .binary_search_by_key(&hash, |&(h, _)| h)
                .ok()
                .map(|i| current[i].1)
        };
        let mut removed: Vec<RemovedEntry> = snapshot
            .entries
            .iter()
            .filter(|e| e.parent != NO_PARENT && find(e.hash).is_none())
            .filter_map(|e| {
                Some(RemovedEntry {
                    parent: find(e.parent)?,
                    name: e.name.clone(),
                    allocated: e.allocated,
                    is_dir: e.is_dir,
                })
            })
            .collect();
        removed.sort_unstable_by(|a, b| {
            a.parent
                .cmp(&b.parent)
                .then(b.allocated.cmp(&a.allocated))
                .then_with(|| a.name.cmp(&b.name))
        });
        Self {
            taken_at: snapshot.taken_at,
            bytes,
            before,
            removed,
        }
    }

    /// When the compared snapshot was taken, seconds since the UNIX epoch.
    pub fn taken_at(&self) -> u32 {
        self.taken_at
    }

    /// Change of node `id` in bytes on disk (0 for trashed nodes).
    pub fn bytes(&self, id: NodeId) -> Option<i64> {
        self.bytes.get(id as usize).copied()
    }

    /// Size of node `id` in the snapshot; `None` when it was not stored there (new, a small
    /// file, or moved to Trash since).
    pub fn before(&self, id: NodeId) -> Option<u64> {
        self.before.get(id as usize).copied().flatten()
    }

    /// Changes of all nodes, indexed by `NodeId`.
    pub fn as_slice(&self) -> &[i64] {
        &self.bytes
    }

    /// Items removed from folder `parent`, largest first.
    pub fn removed_in(&self, parent: NodeId) -> &[RemovedEntry] {
        let start = self.removed.partition_point(|r| r.parent < parent);
        let end = self.removed.partition_point(|r| r.parent <= parent);
        &self.removed[start..end]
    }

    /// All removed items, grouped by folder.
    pub fn removed(&self) -> &[RemovedEntry] {
        &self.removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{EntryMeta, NodeFlags, TreeBuilder, ROOT};
    use std::ffi::OsStr;
    use std::time::Duration;
    use tempfile::TempDir;

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

    struct Fixture {
        tree: ScanTree,
        src: NodeId,
        big: NodeId,
        small: NodeId,
        link: NodeId,
    }

    /// `/data/src/{big.bin 5 MiB, small.txt 10 B}`, `/data/link.bin` (hard link, 3 MiB).
    fn fixture() -> Fixture {
        let mut b = TreeBuilder::new("/data", 7, dir());
        let src = b.push(ROOT, OsStr::new("src"), dir()).unwrap();
        let big = b.push(src, OsStr::new("big.bin"), file(5 * MIB)).unwrap();
        let small = b.push(src, OsStr::new("small.txt"), file(10)).unwrap();
        let link = b
            .push(
                ROOT,
                OsStr::new("link.bin"),
                EntryMeta {
                    flags: NodeFlags::HARDLINK,
                    ..file(3 * MIB)
                },
            )
            .unwrap();
        Fixture {
            tree: b.finish(Duration::ZERO),
            src,
            big,
            small,
            link,
        }
    }

    /// Builds `/data` from `(parent index, name, size or None for a folder)`; index 0 = root.
    fn tree_of(items: &[(usize, &str, Option<u64>)]) -> (ScanTree, Vec<NodeId>) {
        let mut b = TreeBuilder::new("/data", 1, dir());
        let mut ids = vec![ROOT];
        for &(parent, name, size) in items {
            let meta = size.map_or_else(dir, file);
            ids.push(b.push(ids[parent], OsStr::new(name), meta).unwrap());
        }
        (b.finish(Duration::ZERO), ids)
    }

    fn encode(s: &Snapshot) -> Vec<u8> {
        let mut out = Vec::new();
        s.write_to(&mut out).unwrap();
        out
    }

    #[test]
    fn keeps_folders_and_large_files_only() {
        let f = fixture();
        let s = Snapshot::capture(&f.tree);
        let h = path_hashes(&f.tree);
        assert_eq!(s.allocated(h[ROOT as usize]), Some(5 * MIB + 10));
        assert_eq!(s.allocated(h[f.src as usize]), Some(5 * MIB + 10));
        assert_eq!(s.allocated(h[f.big as usize]), Some(5 * MIB));
        assert_eq!(s.allocated(h[f.small as usize]), None);
        // A hard link counted elsewhere frees nothing and adds nothing.
        assert_eq!(s.allocated(h[f.link as usize]), None);
        assert_eq!(s.len(), 3);
        assert_eq!(s.root(), Path::new("/data").to_string_lossy());

        let big = s.get(h[f.big as usize]).unwrap();
        assert_eq!((big.name.as_str(), big.is_dir), ("big.bin", false));
        assert_eq!(big.parent, h[f.src as usize]);
        assert_eq!(s.get(h[ROOT as usize]).unwrap().parent, NO_PARENT);
    }

    #[test]
    fn path_hashes_follow_the_full_path() {
        let f = fixture();
        let h = path_hashes(&f.tree);
        assert_eq!(h.len(), f.tree.len());
        let expected = fnv1a(
            FNV_OFFSET,
            Path::new("/data").as_os_str().as_encoded_bytes(),
        );
        let expected = fnv1a(fnv1a(expected, b"/"), b"src");
        assert_eq!(h[f.src as usize], expected);
        // Same name under another parent hashes differently.
        let mut b = TreeBuilder::new("/other", 1, dir());
        let src = b.push(ROOT, OsStr::new("src"), dir()).unwrap();
        assert_ne!(
            path_hashes(&b.finish(Duration::ZERO))[src as usize],
            expected
        );
    }

    #[test]
    fn trashed_items_and_their_contents_are_left_out() {
        let mut f = fixture();
        let h = path_hashes(&f.tree);
        f.tree.mark_trashed(f.src).unwrap();
        let s = Snapshot::capture(&f.tree);
        assert_eq!(s.allocated(h[f.src as usize]), None);
        assert_eq!(s.allocated(h[f.big as usize]), None);
        assert_eq!(s.allocated(h[ROOT as usize]), Some(0));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn round_trips_through_the_file_format() {
        let f = fixture();
        let s = Snapshot::capture(&f.tree);
        let bytes = encode(&s);
        assert_eq!(&bytes[..4], MAGIC);
        assert_eq!(Snapshot::read_from(bytes.as_slice()).unwrap(), s);
    }

    #[test]
    fn other_format_versions_are_incompatible() {
        let mut bytes = encode(&Snapshot::capture(&fixture().tree));
        for version in [1u16, 99] {
            bytes[4..6].copy_from_slice(&version.to_le_bytes());
            assert!(matches!(
                Snapshot::read_from(bytes.as_slice()),
                Err(SnapshotError::Incompatible { found }) if found == version
            ));
        }
    }

    #[test]
    fn damaged_files_are_reported_without_panicking() {
        let bytes = encode(&Snapshot::capture(&fixture().tree));
        let mut flipped = bytes.clone();
        let last = flipped.len() - 3;
        flipped[last] ^= 0xFF;
        let cases: [&[u8]; 5] = [
            b"",
            b"DSN",
            b"NOPE\x02\x00",
            &bytes[..bytes.len() / 2],
            &flipped,
        ];
        for case in cases {
            assert!(
                matches!(Snapshot::read_from(case), Err(SnapshotError::Corrupt(_))),
                "{case:?}"
            );
        }
        // Valid zstd, but not a snapshot body.
        let mut junk = MAGIC.to_vec();
        junk.extend_from_slice(&SNAPSHOT_FORMAT_VERSION.to_le_bytes());
        junk.extend(zstd::encode_all(&b"\xff\xff\xff"[..], 1).unwrap());
        assert!(matches!(
            Snapshot::read_from(junk.as_slice()),
            Err(SnapshotError::Corrupt(_))
        ));
    }

    #[test]
    fn unsorted_entries_are_corrupt() {
        let entry = |hash| SnapshotEntry {
            hash,
            parent: 0,
            allocated: 1,
            is_dir: true,
            name: "x".into(),
        };
        let body = Body {
            root: "/x".into(),
            taken_at: 0,
            entries: vec![entry(2), entry(1)],
        };
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&SNAPSHOT_FORMAT_VERSION.to_le_bytes());
        bytes.extend(zstd::encode_all(postcard::to_stdvec(&body).unwrap().as_slice(), 1).unwrap());
        assert!(matches!(
            Snapshot::read_from(bytes.as_slice()),
            Err(SnapshotError::Corrupt(_))
        ));
    }

    #[test]
    fn snapshot_of_a_parent_folder_serves_a_folder_scan() {
        let tmp = TempDir::new().unwrap();
        let store = tmp.path().join("snapshots");
        // Whole "drive" first: /data/src/{big.bin 5 MiB, small.txt 10 B}, /data/link.bin.
        let drive = Snapshot::capture(&fixture().tree);
        drive
            .save(&snapshot_path(&store, Path::new("/data")))
            .unwrap();

        // Then only /data/src: big.bin grew, new.bin appeared, small.txt is gone.
        let mut b = TreeBuilder::new("/data/src", 8, dir());
        let big = b.push(ROOT, OsStr::new("big.bin"), file(8 * MIB)).unwrap();
        let new = b.push(ROOT, OsStr::new("new.bin"), file(2 * MIB)).unwrap();
        let folder = b.finish(Duration::ZERO);

        let previous = load_previous(&store, Path::new("/data/src")).unwrap();
        assert_eq!(
            previous.as_ref(),
            Some(&drive),
            "the drive snapshot covers the folder"
        );
        assert!(drive.hash_of(Path::new("/data/missing")).is_none());
        assert!(drive.hash_of(Path::new("/elsewhere/src")).is_none());
        assert_eq!(
            load_previous(&store, Path::new("/data/missing")).unwrap(),
            None
        );

        let delta = Delta::compute(&folder, &drive);
        assert_eq!(delta.bytes(ROOT), Some((5 * MIB - 10) as i64));
        assert_eq!(delta.bytes(big), Some(3 * MIB as i64));
        assert_eq!(delta.bytes(new), Some(2 * MIB as i64));
        // Items outside the scanned folder (link.bin) and the folder itself are not removed.
        assert!(delta.removed().is_empty(), "{:?}", delta.removed());
    }

    #[test]
    fn saves_and_loads_from_a_folder() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("snapshots");
        let f = fixture();
        let root = f.tree.root_path().to_owned();
        assert_eq!(load_previous(&dir, &root).unwrap(), None);

        let s = Snapshot::capture(&f.tree);
        let path = snapshot_path(&dir, &root);
        s.save(&path).unwrap();
        assert!(path.extension() == Some(OsStr::new(SNAPSHOT_EXTENSION)));
        assert!(!path.with_extension("tmp").exists());
        assert_eq!(load_previous(&dir, &root).unwrap(), Some(s.clone()));
        // Another root has its own file.
        assert_eq!(load_previous(&dir, Path::new("/elsewhere")).unwrap(), None);

        // Saving again replaces the previous snapshot.
        let mut f2 = fixture();
        f2.tree.mark_trashed(f2.big).unwrap();
        let s2 = Snapshot::capture(&f2.tree);
        s2.save(&path).unwrap();
        assert_eq!(Snapshot::load(&path).unwrap(), Some(s2));

        fs::write(&path, b"garbage").unwrap();
        assert!(matches!(
            load_previous(&dir, &root),
            Err(SnapshotError::Corrupt(_))
        ));
    }

    #[test]
    fn delta_shows_growth_shrink_and_new_items() {
        // Before: /data/{cache/ (a.bin 4 MiB), docs/ (r.pdf 2 MiB), tiny.txt 10 B}.
        let (before, _) = tree_of(&[
            (0, "cache", None),
            (1, "a.bin", Some(4 * MIB)),
            (0, "docs", None),
            (3, "r.pdf", Some(2 * MIB)),
            (0, "tiny.txt", Some(10)),
        ]);
        let snapshot = Snapshot::capture(&before);
        // After: a.bin grew to 9 MiB, r.pdf shrank to 1.5 MiB, new folder, new small file.
        let (after, ids) = tree_of(&[
            (0, "cache", None),
            (1, "a.bin", Some(9 * MIB)),
            (0, "docs", None),
            (3, "r.pdf", Some(3 * MIB / 2)),
            (0, "tiny.txt", Some(20)),
            (0, "new", None),
            (6, "n.iso", Some(7 * MIB)),
            (0, "note.txt", Some(100)),
        ]);
        let delta = Delta::compute(&after, &snapshot);
        let d = |i: usize| delta.bytes(ids[i]).unwrap();
        let mib = MIB as i64;
        assert_eq!(d(1), 5 * mib);
        assert_eq!(d(2), 5 * mib);
        assert_eq!(d(4), -mib / 2);
        assert_eq!(d(3), -mib / 2);
        assert_eq!((d(6), d(7)), (7 * mib, 7 * mib));
        // Small files are not stored: their change only shows in the folder sums.
        assert_eq!((d(5), d(8)), (0, 0));
        assert_eq!(d(0), 5 * mib - mib / 2 + 7 * mib + 10 + 100);
        assert!(delta.removed().is_empty());
        assert_eq!(delta.taken_at(), snapshot.taken_at());
        assert_eq!(delta.as_slice().len(), after.len());
    }

    #[test]
    fn removed_items_keep_their_name_in_the_closest_folder() {
        let (before, _) = tree_of(&[
            (0, "videos", None),
            (1, "old.mkv", Some(3 * MIB)),
            (1, "keep.mkv", Some(2 * MIB)),
            (0, "build", None),
            (4, "out", None),
            (5, "app.bin", Some(6 * MIB)),
            (1, "clip.mp4", Some(5 * MIB)),
        ]);
        let snapshot = Snapshot::capture(&before);
        let (after, ids) = tree_of(&[(0, "videos", None), (1, "keep.mkv", Some(2 * MIB))]);
        let delta = Delta::compute(&after, &snapshot);

        let videos: Vec<_> = delta
            .removed_in(ids[1])
            .iter()
            .map(|r| (r.name.as_str(), r.allocated, r.is_dir))
            .collect();
        assert_eq!(
            videos,
            [("clip.mp4", 5 * MIB, false), ("old.mkv", 3 * MIB, false)]
        );
        // Only the topmost removed folder is listed; its size includes the contents.
        let root: Vec<_> = delta
            .removed_in(ROOT)
            .iter()
            .map(|r| (r.name.as_str(), r.allocated, r.is_dir))
            .collect();
        assert_eq!(root, [("build", 6 * MIB, true)]);
        assert_eq!(delta.removed().len(), 3);
        assert!(delta.removed_in(ids[2]).is_empty());
        assert_eq!(delta.bytes(ids[1]), Some(-(8 * MIB as i64)));
    }

    #[test]
    fn trashed_items_count_as_removed() {
        let f = fixture();
        let snapshot = Snapshot::capture(&f.tree);
        let mut tree = fixture().tree;
        tree.mark_trashed(f.src).unwrap();
        let delta = Delta::compute(&tree, &snapshot);
        let removed = delta.removed_in(ROOT);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].name, "src");
        assert_eq!(removed[0].allocated, 5 * MIB + 10);
        assert_eq!(delta.bytes(f.src), Some(0));
        assert_eq!(delta.bytes(f.big), Some(0));
        assert_eq!(delta.bytes(ROOT), Some(-((5 * MIB + 10) as i64)));
    }

    #[test]
    fn unchanged_tree_has_no_delta() {
        let f = fixture();
        let delta = Delta::compute(&f.tree, &Snapshot::capture(&f.tree));
        assert!(delta.as_slice().iter().all(|&d| d == 0));
        assert!(delta.removed().is_empty());
        assert_eq!(delta.bytes(9_999), None);
    }
}
