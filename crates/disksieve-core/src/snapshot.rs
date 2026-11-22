//! Scan snapshots for Delta Mode (spec 2.7).
//!
//! A snapshot keeps only folders and files of at least [`MIN_FILE_SIZE`]: a 64-bit hash of the
//! full path and the size on disk. That keeps it small even for millions of nodes — the next
//! scan compares against it to show what grew or shrank.
//!
//! File layout: magic `DSNP`, the format version (`u16`, little endian), then a `zstd` frame
//! with the `postcard`-encoded body. Reading never panics: a file from another format version
//! is [`SnapshotError::Incompatible`], anything unreadable is [`SnapshotError::Corrupt`].

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tree::ScanTree;

/// Current snapshot format; bump it whenever the body layout changes.
pub const SNAPSHOT_FORMAT_VERSION: u16 = 1;
/// Smallest file kept in a snapshot (folders are always kept).
pub const MIN_FILE_SIZE: u64 = 1024 * 1024;
/// Extension of snapshot files.
pub const SNAPSHOT_EXTENSION: &str = "dsnap";

const MAGIC: &[u8; 4] = b"DSNP";
/// Upper bound for a decompressed body, so a damaged file cannot exhaust memory.
const MAX_BODY_BYTES: u64 = 1 << 30;
const ZSTD_LEVEL: i32 = 3;

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

#[derive(Serialize, Deserialize)]
struct Body {
    root: String,
    taken_at: u32,
    /// `(path hash, size on disk)`, sorted by hash, hashes unique.
    entries: Vec<(u64, u64)>,
}

/// Sizes of one scan, keyed by path hash (see [`path_hashes`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    root: String,
    taken_at: u32,
    entries: Vec<(u64, u64)>,
}

impl Snapshot {
    /// Captures folders and files ≥ [`MIN_FILE_SIZE`] of `tree` (items moved to Trash are left
    /// out). Sizes are what each item adds to its parent, so hard links count once.
    pub fn capture(tree: &ScanTree) -> Self {
        let hashes = path_hashes(tree);
        let mut entries: Vec<(u64, u64)> = (0..tree.len())
            .filter_map(|i| {
                let node = tree.get(i as u32)?;
                let (_, allocated, _) = node.contribution();
                let keep = !node.is_trashed() && (node.is_dir() || allocated >= MIN_FILE_SIZE);
                keep.then(|| (hashes[i], allocated))
            })
            .collect();
        entries.sort_unstable_by_key(|&(hash, _)| hash);
        // Two paths with the same 64-bit hash are vanishingly rare; keep the first.
        entries.dedup_by_key(|&mut (hash, _)| hash);
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

    /// Number of stored items.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Size on disk of the item with this path hash, if it was stored.
    pub fn allocated(&self, path_hash: u64) -> Option<u64> {
        self.entries
            .binary_search_by_key(&path_hash, |&(hash, _)| hash)
            .ok()
            .map(|i| self.entries[i].1)
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
        if !body.entries.windows(2).all(|w| w[0].0 < w[1].0) {
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

/// Latest snapshot of `root` stored in `dir`. A snapshot of another root (hash collision of
/// the file name) counts as none.
pub fn load_previous(dir: &Path, root: &Path) -> Result<Option<Snapshot>, SnapshotError> {
    let snapshot = Snapshot::load(&snapshot_path(dir, root))?;
    Ok(snapshot.filter(|s| s.root == root.to_string_lossy()))
}

/// 64-bit hash of every node's full path, indexed by `NodeId`. Stable across scans and app
/// versions (FNV-1a), so it can key a snapshot.
pub fn path_hashes(tree: &ScanTree) -> Vec<u64> {
    let mut hashes = Vec::with_capacity(tree.len());
    hashes.push(fnv1a(
        FNV_OFFSET,
        tree.root_path().as_os_str().as_encoded_bytes(),
    ));
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

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{EntryMeta, NodeFlags, NodeId, TreeBuilder, ROOT};
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
    fn trashed_items_are_left_out() {
        let mut f = fixture();
        let h = path_hashes(&f.tree);
        f.tree.mark_trashed(f.src).unwrap();
        let s = Snapshot::capture(&f.tree);
        assert_eq!(s.allocated(h[f.src as usize]), None);
        assert_eq!(s.allocated(h[ROOT as usize]), Some(0));
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
        bytes[4..6].copy_from_slice(&99u16.to_le_bytes());
        assert!(matches!(
            Snapshot::read_from(bytes.as_slice()),
            Err(SnapshotError::Incompatible { found: 99 })
        ));
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
            b"NOPE\x01\x00",
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
        let body = Body {
            root: "/x".into(),
            taken_at: 0,
            entries: vec![(2, 1), (1, 1)],
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
}
