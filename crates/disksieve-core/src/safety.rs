//! Delete guard and pre-trash verification (spec 2.5).
//!
//! The only way DiskSieve removes anything is moving it to the system Trash, and only after
//! three checks that the UI cannot bypass:
//!
//! 1. [`Guard::check`] — the node itself is allowed: not the scan root, not a link, cloud
//!    placeholder or another volume, and its path is not protected (volume roots, the home
//!    folder and its ancestors, system locations and files, DiskSieve's own folders);
//! 2. [`verify_unchanged`] — the entry on disk is still the one that was scanned (same type,
//!    size and modification time);
//! 3. the move itself; only after it succeeds is the tree updated with
//!    [`ScanTree::mark_trashed`], so no rescan is needed.
//!
//! The Trash operation is passed in as a closure: production code uses
//! [`move_to_system_trash`] / [`restore_from_system_trash`], tests use a fake and never touch
//! the real Trash.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::CoreError;
use crate::tree::{to_unix_secs, Node, NodeFlags, NodeId, ScanTree, ROOT};

/// Why the guard refuses to move an entry to Trash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("the scanned folder itself cannot be moved to Trash")]
    ScanRoot,
    #[error("already moved to Trash")]
    AlreadyTrashed,
    #[error("the root of a drive is protected")]
    VolumeRoot,
    #[error("your home folder and the folders containing it are protected")]
    HomeDirectory,
    #[error("system location — removing it could break the operating system or installed apps")]
    SystemLocation,
    #[error("system file — managed by the operating system")]
    SystemFile,
    #[error("DiskSieve's own files are protected")]
    AppDirectory,
    #[error("online-only cloud file — remove it in the cloud app instead")]
    CloudOnline,
    #[error("links, junctions and mount points are not moved to Trash")]
    Link,
    #[error("it belongs to another drive")]
    OtherVolume,
    #[error("the path contains a non-Unicode name and cannot be resolved exactly")]
    UnrepresentablePath,
}

/// Failure of one entry in a bulk trash / restore operation.
#[derive(Debug, thiserror::Error)]
pub enum TrashError {
    #[error("node {0} does not exist in the scan tree")]
    InvalidNode(NodeId),
    #[error("protected: {0}")]
    Refused(#[from] Refusal),
    #[error("changed since the scan ({0}) — rescan and try again")]
    Changed(&'static str),
    #[error("no longer exists")]
    Missing,
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Totals that one entry adds to its parent, as moved to (or restored from) Trash.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Freed {
    pub logical: u64,
    pub allocated: u64,
    pub files: u32,
}

impl Freed {
    fn of(node: &Node) -> Self {
        let (logical, allocated, files) = node.contribution();
        Self {
            logical,
            allocated,
            files,
        }
    }
}

/// Result for one requested node.
#[derive(Debug)]
pub struct TrashOutcome {
    pub id: NodeId,
    /// Display path (may be lossy); empty for an invalid id.
    pub path: String,
    pub result: Result<Freed, TrashError>,
}

/// Whether [`restore_from_system_trash`] works on this platform (the `trash` crate can list
/// and restore items on Windows and freedesktop Linux, not on macOS).
pub const RESTORE_SUPPORTED: bool = cfg!(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
));

/// Windows and macOS file systems are case-insensitive by default.
const FOLD_CASE: bool = cfg!(any(windows, target_os = "macos"));

/// Files the OS manages itself, refused wherever they are.
const SYSTEM_FILES: &[&str] = &["hiberfil.sys", "pagefile.sys", "swapfile.sys"];

/// Folders directly under any drive root that belong to Windows (with their contents).
#[cfg(windows)]
const VOLUME_SYSTEM_DIRS: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "system volume information",
    "$recycle.bin",
    "recovery",
    "boot",
    "config.msi",
];
#[cfg(not(windows))]
const VOLUME_SYSTEM_DIRS: &[&str] = &[];

/// Unix system trees: the folder, its contents and its ancestors are refused.
#[cfg(unix)]
const UNIX_SYSTEM_TREES: &[&str] = &[
    "/bin", "/boot", "/dev", "/etc", "/lib", "/lib32", "/lib64", "/libx32", "/proc", "/run",
    "/sbin", "/sys", "/usr", "/System", "/private", "/cores",
];
/// Top-level Unix folders that are refused themselves; their contents are not.
#[cfg(unix)]
const UNIX_SYSTEM_DIRS: &[&str] = &[
    "/home",
    "/Users",
    "/root",
    "/var",
    "/opt",
    "/srv",
    "/tmp",
    "/mnt",
    "/media",
    "/Volumes",
    "/Applications",
    "/Library",
];

#[derive(Debug, Clone)]
struct Protected {
    components: Vec<String>,
    /// `true`: everything inside is refused too; `false`: only the folder and its ancestors.
    subtree: bool,
    refusal: Refusal,
}

/// Decides which paths may be moved to Trash. Pure path logic — no filesystem access.
#[derive(Debug, Clone)]
pub struct Guard {
    protected: Vec<Protected>,
    fold_case: bool,
}

impl Guard {
    /// Guard with the built-in system locations of this OS plus the user's home folder and
    /// DiskSieve's own folders (install, data, cache — protected with their contents).
    pub fn new(home: Option<PathBuf>, app_dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut guard = Self::empty(FOLD_CASE);
        guard.add_system_locations();
        if let Some(home) = home {
            guard.protect(&home, false, Refusal::HomeDirectory);
        }
        for dir in app_dirs {
            guard.protect(&dir, true, Refusal::AppDirectory);
        }
        guard
    }

    fn empty(fold_case: bool) -> Self {
        Self {
            protected: Vec::new(),
            fold_case,
        }
    }

    fn protect(&mut self, path: &Path, subtree: bool, refusal: Refusal) {
        if path.as_os_str().is_empty() {
            return;
        }
        self.protected.push(Protected {
            components: components(path, self.fold_case),
            subtree,
            refusal,
        });
    }

    #[cfg(windows)]
    fn add_system_locations(&mut self) {
        let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
        for name in [
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
        ] {
            if let Some(p) = env(name) {
                self.protect(&p, true, Refusal::SystemLocation);
            }
        }
        for name in ["ProgramData", "PUBLIC"] {
            if let Some(p) = env(name) {
                self.protect(&p, false, Refusal::SystemLocation);
            }
        }
    }

    #[cfg(unix)]
    fn add_system_locations(&mut self) {
        for p in UNIX_SYSTEM_TREES {
            self.protect(Path::new(p), true, Refusal::SystemLocation);
        }
        for p in UNIX_SYSTEM_DIRS {
            self.protect(Path::new(p), false, Refusal::SystemLocation);
        }
    }

    #[cfg(not(any(windows, unix)))]
    fn add_system_locations(&mut self) {}

    /// Checks an absolute path against the protected locations.
    pub fn check_path(&self, path: &Path) -> Result<(), Refusal> {
        if path.parent().is_none() {
            return Err(Refusal::VolumeRoot);
        }
        let name = path
            .file_name()
            .map(|n| fold(&n.to_string_lossy(), self.fold_case));
        if name.as_deref().is_some_and(|n| SYSTEM_FILES.contains(&n)) {
            return Err(Refusal::SystemFile);
        }
        if path.has_root() {
            let first = path
                .components()
                .find(|c| matches!(c, Component::Normal(_)))
                .map(|c| fold(&c.as_os_str().to_string_lossy(), self.fold_case));
            if first
                .as_deref()
                .is_some_and(|f| VOLUME_SYSTEM_DIRS.contains(&f))
            {
                return Err(Refusal::SystemLocation);
            }
        }
        let comps = components(path, self.fold_case);
        for p in &self.protected {
            let inside = comps.starts_with(&p.components);
            let is_it_or_ancestor = p.components.starts_with(&comps);
            if is_it_or_ancestor || (inside && p.subtree) {
                return Err(p.refusal);
            }
        }
        Ok(())
    }

    /// Checks node `id` and returns its exact path when it may be moved to Trash.
    pub fn check(&self, tree: &ScanTree, id: NodeId) -> Result<PathBuf, TrashError> {
        let node = tree.get(id).ok_or(TrashError::InvalidNode(id))?;
        if id == ROOT {
            return Err(Refusal::ScanRoot.into());
        }
        if node.is_trashed()
            || tree
                .ancestors(id)
                .any(|a| tree.get(a).is_some_and(Node::is_trashed))
        {
            return Err(Refusal::AlreadyTrashed.into());
        }
        let refusal = if node.flags.contains(NodeFlags::CLOUD_ONLINE) {
            Some(Refusal::CloudOnline)
        } else if node.flags.contains(NodeFlags::REPARSE) {
            Some(Refusal::Link)
        } else if node.flags.contains(NodeFlags::OTHER_VOLUME) {
            Some(Refusal::OtherVolume)
        } else {
            None
        };
        if let Some(r) = refusal {
            return Err(r.into());
        }
        let path = tree.resolve_path(id).map_err(|e| match e {
            CoreError::InvalidNode(id) => TrashError::InvalidNode(id),
            _ => Refusal::UnrepresentablePath.into(),
        })?;
        self.check_path(&path)?;
        Ok(path)
    }
}

fn fold(s: &str, fold_case: bool) -> String {
    if fold_case {
        s.to_lowercase()
    } else {
        s.to_owned()
    }
}

/// Path components as comparable strings (`C:` prefix, root, names).
fn components(path: &Path, fold_case: bool) -> Vec<String> {
    path.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| match c {
            Component::RootDir => "/".to_owned(),
            other => fold(&other.as_os_str().to_string_lossy(), fold_case),
        })
        .collect()
}

/// Confirms the entry at `path` is still the one node `id` describes: same kind, and for a
/// file the same size and mtime. For a directory only its own mtime is compared with the
/// scan start (entries added, removed or renamed directly inside it), unless DiskSieve
/// changed it itself; deeper changes need a rescan to detect.
pub fn verify_unchanged(tree: &ScanTree, id: NodeId, path: &Path) -> Result<(), TrashError> {
    let node = tree.get(id).ok_or(TrashError::InvalidNode(id))?;
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(TrashError::Missing),
        Err(e) => return Err(e.into()),
    };
    if meta.file_type().is_symlink() {
        return Err(Refusal::Link.into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        const OFFLINE: u32 = 0x1000;
        const RECALL_ON_OPEN: u32 = 0x4_0000;
        const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
        let attrs = meta.file_attributes();
        if attrs & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0 {
            return Err(Refusal::CloudOnline.into());
        }
        if attrs & REPARSE_POINT != 0 {
            return Err(Refusal::Link.into());
        }
    }
    if meta.is_dir() != node.is_dir() {
        return Err(TrashError::Changed("file and folder swapped"));
    }
    let mtime = meta.modified().map(to_unix_secs).unwrap_or(0);
    if node.is_dir() {
        if mtime > tree.stats().started_at && !tree.is_touched(id) {
            return Err(TrashError::Changed("its contents were modified"));
        }
    } else if meta.len() != node.logical_size {
        return Err(TrashError::Changed("size differs"));
    } else if mtime != node.newest_mtime {
        return Err(TrashError::Changed("modification time differs"));
    }
    Ok(())
}

/// Moves each node to Trash through `mover` after [`Guard::check`] and
/// [`verify_unchanged`]; updates the tree after every success. One failure never stops the
/// rest. Duplicate ids are processed once.
pub fn trash_nodes(
    tree: &mut ScanTree,
    ids: &[NodeId],
    guard: &Guard,
    mut mover: impl FnMut(&Path) -> io::Result<()>,
) -> Vec<TrashOutcome> {
    let mut outcomes = Vec::with_capacity(ids.len());
    for (i, &id) in ids.iter().enumerate() {
        if ids[..i].contains(&id) {
            continue;
        }
        let result = guard.check(tree, id).and_then(|path| {
            verify_unchanged(tree, id, &path)?;
            let freed = Freed::of(tree.node(id).map_err(|_| TrashError::InvalidNode(id))?);
            mover(&path)?;
            tree.mark_trashed(id)
                .map_err(|_| TrashError::Refused(Refusal::AlreadyTrashed))?;
            Ok(freed)
        });
        outcomes.push(TrashOutcome {
            id,
            path: tree.display_path(id),
            result,
        });
    }
    outcomes
}

/// Restores nodes trashed in this session through `restorer` and adds them back to the
/// tree. Refused when something else now exists at the original path.
pub fn restore_nodes(
    tree: &mut ScanTree,
    ids: &[NodeId],
    mut restorer: impl FnMut(&Path) -> io::Result<()>,
) -> Vec<TrashOutcome> {
    let mut outcomes = Vec::with_capacity(ids.len());
    for (i, &id) in ids.iter().enumerate() {
        if ids[..i].contains(&id) {
            continue;
        }
        let result = (|| {
            let node = tree.get(id).ok_or(TrashError::InvalidNode(id))?;
            if !node.is_trashed() {
                return Err(TrashError::Changed("it is not in Trash"));
            }
            let path = tree
                .resolve_path(id)
                .map_err(|_| Refusal::UnrepresentablePath)?;
            if fs::symlink_metadata(&path).is_ok() {
                return Err(TrashError::Changed(
                    "something else now exists at this path",
                ));
            }
            restorer(&path)?;
            tree.restore_trashed(id)
                .map_err(|_| TrashError::Changed("a parent folder is in Trash"))?;
            Ok(Freed::of(
                tree.node(id).map_err(|_| TrashError::InvalidNode(id))?,
            ))
        })();
        outcomes.push(TrashOutcome {
            id,
            path: tree.display_path(id),
            result,
        });
    }
    outcomes
}

/// Moves `path` to the system Trash / Recycle Bin.
pub fn move_to_system_trash(path: &Path) -> io::Result<()> {
    trash::delete(path).map_err(|e| io::Error::other(format!("could not move to Trash: {e}")))
}

/// Restores the most recently trashed item whose original location is `path`.
#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
pub fn restore_from_system_trash(path: &Path) -> io::Result<()> {
    let items = trash::os_limited::list()
        .map_err(|e| io::Error::other(format!("could not read the Trash: {e}")))?;
    let item = latest_deleted(items, path, |i| {
        (i.original_parent.join(&i.name), i.time_deleted)
    })
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not found in Trash"))?;
    trash::os_limited::restore_all([item])
        .map_err(|e| io::Error::other(format!("could not restore from Trash: {e}")))
}

/// Restoring is not available on this platform (see [`RESTORE_SUPPORTED`]).
#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
pub fn restore_from_system_trash(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "restoring from Trash is not supported on this platform",
    ))
}

/// The item with original location `path` deleted most recently.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn latest_deleted<T>(
    items: impl IntoIterator<Item = T>,
    path: &Path,
    key: impl Fn(&T) -> (PathBuf, i64),
) -> Option<T> {
    let wanted = components(path, FOLD_CASE);
    items
        .into_iter()
        .map(|item| {
            let (original, time) = key(&item);
            (item, original, time)
        })
        .filter(|(_, original, _)| components(original, FOLD_CASE) == wanted)
        .max_by_key(|(_, _, time)| *time)
        .map(|(item, _, _)| item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scan, CancelToken, ScanOptions};
    use std::cell::RefCell;
    use tempfile::TempDir;

    fn scanned(dir: &TempDir) -> ScanTree {
        scan(
            dir.path(),
            &ScanOptions::default(),
            &CancelToken::new(),
            |_| {},
        )
        .unwrap()
    }

    /// Fake Trash: moves the entry into a separate temp folder, like the real one would.
    struct FakeTrash {
        bin: TempDir,
        moved: RefCell<Vec<(PathBuf, PathBuf)>>,
    }

    impl FakeTrash {
        fn new() -> Self {
            Self {
                bin: TempDir::new().unwrap(),
                moved: RefCell::default(),
            }
        }
        fn delete(&self, path: &Path) -> io::Result<()> {
            let n = self.moved.borrow().len();
            let target = self.bin.path().join(n.to_string());
            fs::rename(path, &target)?;
            self.moved.borrow_mut().push((path.to_owned(), target));
            Ok(())
        }
        fn restore(&self, path: &Path) -> io::Result<()> {
            let target = latest_deleted(
                self.moved.borrow().iter().enumerate(),
                path,
                |(i, (p, _))| (p.clone(), *i as i64),
            )
            .map(|(_, (_, t))| t.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            fs::rename(target, path)
        }
    }

    fn id_of(tree: &ScanTree, rel: &str) -> NodeId {
        rel.split('/')
            .try_fold(ROOT, |id, name| tree.child_by_name(id, name))
            .unwrap_or_else(|| panic!("{rel} not in tree"))
    }

    fn guard_without_home() -> Guard {
        Guard::new(None, [])
    }

    #[test]
    fn trash_moves_entries_and_updates_tree() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("cache")).unwrap();
        fs::write(dir.path().join("cache/blob.bin"), vec![0u8; 10_000]).unwrap();
        fs::write(dir.path().join("keep.txt"), b"keep").unwrap();
        let mut tree = scanned(&dir);
        let cache = id_of(&tree, "cache");
        let before = tree.node(ROOT).unwrap().file_count;

        let fake = FakeTrash::new();
        let out = trash_nodes(&mut tree, &[cache, cache], &guard_without_home(), |p| {
            fake.delete(p)
        });

        assert_eq!(out.len(), 1, "duplicates are processed once");
        let freed = out[0].result.as_ref().unwrap();
        assert_eq!((freed.logical, freed.files), (10_000, 1));
        assert!(!dir.path().join("cache").exists());
        assert!(dir.path().join("keep.txt").exists());
        assert!(tree.node(cache).unwrap().is_trashed());
        assert_eq!(tree.node(ROOT).unwrap().file_count, before - 1);
    }

    #[test]
    fn one_failure_does_not_stop_the_rest() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), b"a").unwrap();
        fs::write(dir.path().join("b.txt"), b"b").unwrap();
        let mut tree = scanned(&dir);
        let (a, b) = (id_of(&tree, "a.txt"), id_of(&tree, "b.txt"));
        fs::remove_file(dir.path().join("a.txt")).unwrap();

        let fake = FakeTrash::new();
        let out = trash_nodes(
            &mut tree,
            &[ROOT, a, 9_999, b],
            &guard_without_home(),
            |p| fake.delete(p),
        );
        assert!(matches!(
            out[0].result,
            Err(TrashError::Refused(Refusal::ScanRoot))
        ));
        assert!(matches!(out[1].result, Err(TrashError::Missing)));
        assert!(matches!(out[2].result, Err(TrashError::InvalidNode(9_999))));
        assert!(out[3].result.is_ok());
        assert!(
            !tree.node(a).unwrap().is_trashed(),
            "failed entries stay in the tree"
        );
    }

    #[test]
    fn file_changed_since_scan_is_not_trashed() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("log.txt");
        fs::write(&file, b"short").unwrap();
        let mut tree = scanned(&dir);
        let id = id_of(&tree, "log.txt");
        fs::write(&file, b"much longer content").unwrap();

        let out = trash_nodes(&mut tree, &[id], &guard_without_home(), |_| {
            panic!("must not reach the Trash")
        });
        assert!(
            matches!(out[0].result, Err(TrashError::Changed(_))),
            "{:?}",
            out[0].result
        );
        assert!(file.exists());
    }

    #[test]
    fn folder_with_new_entries_since_scan_is_not_trashed() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("docs")).unwrap();
        let mut tree = scanned(&dir);
        let docs = id_of(&tree, "docs");
        // Pretend the scan ran a while ago, then add a file to the folder.
        tree_started_at(&mut tree, 1);
        fs::write(dir.path().join("docs/new.txt"), b"new").unwrap();

        let err = verify_unchanged(&tree, docs, &dir.path().join("docs")).unwrap_err();
        assert!(matches!(err, TrashError::Changed(_)), "{err:?}");
    }

    #[test]
    fn folder_changed_by_disksieve_itself_can_still_be_trashed() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build/a.o"), b"obj").unwrap();
        let mut tree = scanned(&dir);
        tree_started_at(&mut tree, 1);
        let (build, obj) = (id_of(&tree, "build"), id_of(&tree, "build/a.o"));
        let fake = FakeTrash::new();
        let guard = guard_without_home();
        // `a.o` keeps its mtime, so it passes; trashing it updates the folder's mtime.
        let out = trash_nodes(&mut tree, &[obj], &guard, |p| fake.delete(p));
        assert!(out[0].result.is_ok(), "{:?}", out[0].result);
        let out = trash_nodes(&mut tree, &[build], &guard, |p| fake.delete(p));
        assert!(out[0].result.is_ok(), "{:?}", out[0].result);
    }

    fn tree_started_at(tree: &mut ScanTree, secs: u32) {
        tree.stats_mut().started_at = secs;
    }

    #[test]
    fn restore_brings_entry_back_and_updates_tree() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("photo.jpg"), vec![1u8; 5_000]).unwrap();
        let mut tree = scanned(&dir);
        let id = id_of(&tree, "photo.jpg");
        let total = tree.node(ROOT).unwrap().logical_size;
        let fake = FakeTrash::new();

        trash_nodes(&mut tree, &[id], &guard_without_home(), |p| fake.delete(p));
        assert_eq!(tree.node(ROOT).unwrap().logical_size, total - 5_000);

        let out = restore_nodes(&mut tree, &[id], |p| fake.restore(p));
        assert_eq!(out[0].result.as_ref().unwrap().logical, 5_000);
        assert!(dir.path().join("photo.jpg").exists());
        assert_eq!(tree.node(ROOT).unwrap().logical_size, total);

        let again = restore_nodes(&mut tree, &[id], |p| fake.restore(p));
        assert!(matches!(again[0].result, Err(TrashError::Changed(_))));
    }

    #[test]
    fn restore_refuses_when_path_is_taken() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, b"old").unwrap();
        let mut tree = scanned(&dir);
        let id = id_of(&tree, "a.txt");
        let fake = FakeTrash::new();
        trash_nodes(&mut tree, &[id], &guard_without_home(), |p| fake.delete(p));
        fs::write(&file, b"new file with the same name").unwrap();

        let out = restore_nodes(&mut tree, &[id], |_| panic!("must not restore"));
        assert!(matches!(out[0].result, Err(TrashError::Changed(_))));
        assert_eq!(fs::read(&file).unwrap(), b"new file with the same name");
    }

    #[test]
    fn home_and_its_ancestors_are_refused_but_contents_allowed() {
        let home = TempDir::new().unwrap();
        let guard = Guard::new(Some(home.path().to_owned()), []);
        assert_eq!(guard.check_path(home.path()), Err(Refusal::HomeDirectory));
        assert_eq!(
            guard.check_path(home.path().parent().unwrap()),
            Err(Refusal::HomeDirectory)
        );
        assert_eq!(guard.check_path(&home.path().join("Downloads")), Ok(()));
    }

    #[test]
    fn app_directories_are_refused_with_contents_and_ancestors() {
        let base = TempDir::new().unwrap();
        let data = base.path().join("com.disksieve.app");
        let guard = Guard::new(None, [data.clone()]);
        assert_eq!(guard.check_path(&data), Err(Refusal::AppDirectory));
        assert_eq!(
            guard.check_path(&data.join("snapshots")),
            Err(Refusal::AppDirectory)
        );
        assert_eq!(guard.check_path(base.path()), Err(Refusal::AppDirectory));
        assert_eq!(guard.check_path(&base.path().join("other")), Ok(()));
    }

    #[test]
    fn volume_roots_and_system_files_are_refused() {
        let guard = guard_without_home();
        let root = if cfg!(windows) { r"C:\" } else { "/" };
        assert_eq!(guard.check_path(Path::new(root)), Err(Refusal::VolumeRoot));
        let dir = TempDir::new().unwrap();
        assert_eq!(
            guard.check_path(&dir.path().join("pagefile.sys")),
            Err(Refusal::SystemFile)
        );
        if FOLD_CASE {
            assert_eq!(
                guard.check_path(&dir.path().join("HIBERFIL.SYS")),
                Err(Refusal::SystemFile)
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_system_locations_are_refused() {
        let guard = guard_without_home();
        for p in [
            r"C:\Windows",
            r"C:\WINDOWS\System32\drivers",
            r"D:\Program Files\App",
            r"C:\Program Files (x86)",
            r"C:\System Volume Information",
            r"E:\$Recycle.Bin\S-1-5-21",
        ] {
            assert_eq!(
                guard.check_path(Path::new(p)),
                Err(Refusal::SystemLocation),
                "{p}"
            );
        }
        assert_eq!(guard.check_path(Path::new(r"C:\Games\Windows")), Ok(()));
        assert_eq!(
            guard.check_path(Path::new(r"D:\Projects\app\target")),
            Ok(())
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_system_locations_are_refused() {
        let guard = guard_without_home();
        for p in [
            "/usr",
            "/usr/lib/libc.so",
            "/etc/hosts",
            "/System/Library",
            "/home",
            "/var",
        ] {
            assert_eq!(
                guard.check_path(Path::new(p)),
                Err(Refusal::SystemLocation),
                "{p}"
            );
        }
        assert_eq!(guard.check_path(Path::new("/var/cache/apt")), Ok(()));
        assert_eq!(guard.check_path(Path::new("/opt/tool/cache")), Ok(()));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_refused() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("target.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("target.txt"), dir.path().join("link")).unwrap();
        let mut tree = scanned(&dir);
        let link = id_of(&tree, "link");
        let out = trash_nodes(&mut tree, &[link], &guard_without_home(), |_| {
            panic!("must not reach the Trash")
        });
        assert!(matches!(
            out[0].result,
            Err(TrashError::Refused(Refusal::Link))
        ));
        assert!(dir.path().join("target.txt").exists());
    }

    #[test]
    fn latest_deleted_picks_newest_matching_item() {
        let p = std::env::temp_dir().join("x").join("a.txt");
        let other = std::env::temp_dir().join("x").join("b.txt");
        let items = vec![
            (p.clone(), 10, "old"),
            (other, 30, "other"),
            (p.clone(), 20, "new"),
        ];
        let found = latest_deleted(items, &p, |(path, t, _)| (path.clone(), *t));
        assert_eq!(found.map(|i| i.2), Some("new"));
    }
}
