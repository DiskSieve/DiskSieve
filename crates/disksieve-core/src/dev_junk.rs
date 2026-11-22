//! Dev Junk: rebuildable project artifacts (spec 2.4).
//!
//! A folder counts as an artifact **only** when the project marker next to it confirms it —
//! `build/` is junk beside `package.json` or `build.gradle`, not on its own. Detection works on
//! the scan tree alone (no disk access): markers are looked up among the parent's children.
//!
//! - [`detect_junk`] marks the top-most artifacts in the tree (`Node::junk`); nothing inside an
//!   artifact is examined, so `node_modules/x/node_modules` or `node_modules/x/dist` count once.
//!   Installed software is never searched ([`is_installed_software`]): a `dist/` next to
//!   `package.json` inside a VS Code extension or a global npm package is the program itself.
//! - [`junk_projects`] groups the marked artifacts by project, with the project's age taken
//!   from its newest file **outside** artifacts (a fresh `npm install` does not make an old
//!   project look active).

use std::collections::{BTreeMap, HashSet};

use crate::tree::{Node, NodeFlags, NodeId, ScanTree, NO_NODE, ROOT};

/// High bit of `Node::junk`: an artifact lies somewhere below this folder.
const CONTAINS_JUNK: u8 = 0x80;
/// Low bits of `Node::junk`: the `JunkKind` code.
const KIND_MASK: u8 = 0x7F;

/// Folders holding installed software, app data or tool caches. Detection does not search
/// inside them: their `package.json` + `dist/` pairs are shipped programs, not projects.
/// Global tool caches get their own list later (spec step 7.5). Compared ignoring case.
const INSTALLED_SOFTWARE: &[&str] = &[
    // Installed packages and the folders that hold them.
    "node_modules",
    "site-packages",
    "dist-packages",
    "bower_components",
    // Editors and their extensions.
    ".vscode",
    ".vscode-insiders",
    ".vscode-server",
    ".cursor",
    ".windsurf",
    // Per-user app data and tool homes.
    "AppData",
    ".cargo",
    ".rustup",
    ".npm",
    ".nvm",
    ".pnpm-store",
    ".yarn",
    ".bun",
    ".deno",
    ".m2",
    ".ivy2",
    ".sbt",
    ".nuget",
    ".gem",
    ".pyenv",
    ".conda",
    "anaconda3",
    "miniconda3",
    ".cache",
    ".local",
    ".dotnet",
    ".android",
    ".gradle",
    // Version control internals.
    ".git",
    ".hg",
    ".svn",
    // System and program folders.
    "Windows",
    "Program Files",
    "Program Files (x86)",
    "ProgramData",
    "$Recycle.Bin",
    "System Volume Information",
    ".Trash",
    ".Trashes",
    "Applications",
    "System",
];

/// Whether `dir` holds installed software rather than projects (see [`INSTALLED_SOFTWARE`]).
/// macOS app bundles (`*.app`) count too, as do the per-user `Library` and `snap` folders.
fn is_installed_software(name: &str) -> bool {
    INSTALLED_SOFTWARE
        .iter()
        .any(|s| s.eq_ignore_ascii_case(name))
        || name.len() > 4
            && name
                .get(name.len() - 4..)
                .is_some_and(|s| s.eq_ignore_ascii_case(".app"))
        || name == "Library"
        || name == "snap"
}

/// Defines [`JunkKind`] with its `u8` codes (stored in `Node::junk`) and stable names.
/// New kinds are only ever appended, so codes stay stable.
macro_rules! junk_kinds {
    ($($kind:ident = $code:literal => $name:literal,)+) => {
        /// Kind of rebuildable artifact. Stored in [`Node::junk`] as `u8` (0 = not junk).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum JunkKind {
            $($kind = $code,)+
        }

        impl JunkKind {
            pub const ALL: &'static [JunkKind] = &[$(Self::$kind,)+];

            /// Stable identifier used in the UI and IPC, e.g. `node_modules`.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$kind => $name,)+
                }
            }
        }
    };
}

junk_kinds! {
    NodeModules = 1 => "node_modules",
    NextBuild = 2 => "next_build",
    JsBuild = 3 => "js_build",
    CargoTarget = 4 => "cargo_target",
    PythonVenv = 5 => "python_venv",
    PythonCache = 6 => "python_cache",
    GradleBuild = 7 => "gradle_build",
    MavenTarget = 8 => "maven_target",
    DotnetBuild = 9 => "dotnet_build",
    JsToolCache = 10 => "js_tool_cache",
    PythonBuild = 11 => "python_build",
    PhpVendor = 12 => "php_vendor",
    RubyBundle = 13 => "ruby_bundle",
    DartBuild = 14 => "dart_build",
    SwiftBuild = 15 => "swift_build",
    CmakeBuild = 16 => "cmake_build",
    ElixirBuild = 17 => "elixir_build",
    HaskellBuild = 18 => "haskell_build",
    ScalaBuild = 19 => "scala_build",
    ClojureBuild = 20 => "clojure_build",
    ZigBuild = 21 => "zig_build",
    TerraformCache = 22 => "terraform_cache",
    UnityCache = 23 => "unity_cache",
    UnrealCache = 24 => "unreal_cache",
    GodotCache = 25 => "godot_cache",
}

impl JunkKind {
    /// Inverse of `kind as u8`; `None` for 0 and unknown values.
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| *k as u8 == v)
    }

    /// Kind of node `id`, if [`detect_junk`] marked it.
    pub fn of(tree: &ScanTree, id: NodeId) -> Option<Self> {
        tree.get(id).and_then(|n| Self::from_u8(n.junk & KIND_MASK))
    }

    /// How the artifact comes back after it is deleted (shown by the Safety Advisor).
    pub fn rebuild_hint(self) -> &'static str {
        match self {
            Self::NodeModules => "Dependencies of a JavaScript project. Reinstall them with `npm install`, `pnpm install` or `yarn`.",
            Self::NextBuild => "Framework build output (.next, .nuxt, .svelte-kit). The next `dev` or `build` run recreates it.",
            Self::JsBuild => "Build output of a JavaScript project. The project's build script recreates it.",
            Self::JsToolCache => "Tool caches and coverage reports of a JavaScript project, recreated on the next run.",
            Self::CargoTarget => "Rust build output. `cargo build` recreates it; the next build is a full one.",
            Self::PythonVenv => "Python virtual environment. Recreate it with `python -m venv .venv` and reinstall the dependencies.",
            Self::PythonCache => "Python bytecode and tool caches, recreated automatically.",
            Self::PythonBuild => "Python build output. `pip install .`, `python -m build` or tox recreate it.",
            Self::GradleBuild => "Gradle build output, recreated by the next build or sync.",
            Self::MavenTarget => "Maven build output. `mvn package` recreates it.",
            Self::DotnetBuild => ".NET build output (bin/obj). `dotnet build` recreates it.",
            Self::PhpVendor => "PHP dependencies. `composer install` restores them.",
            Self::RubyBundle => "Ruby gems installed for the project. `bundle install` restores them.",
            Self::DartBuild => "Dart / Flutter build output. `flutter pub get` and `flutter build` recreate it.",
            Self::SwiftBuild => "Swift build output or CocoaPods. `swift build` or `pod install` recreate it.",
            Self::CmakeBuild => "CMake build folder. Re-run the CMake configure and build steps.",
            Self::ElixirBuild => "Elixir build output and dependencies. `mix deps.get` and `mix compile` recreate them.",
            Self::HaskellBuild => "Haskell build output. `stack build` or `cabal build` recreate it.",
            Self::ScalaBuild => "Scala build output, recreated by the next sbt build or Metals import.",
            Self::ClojureBuild => "Clojure build output, recreated by the next build or REPL start.",
            Self::ZigBuild => "Zig build cache and output. `zig build` recreates them.",
            Self::TerraformCache => "Terraform providers and modules. `terraform init` restores them.",
            Self::UnityCache => "Unity project cache. Unity rebuilds it when the project is opened, which can take a while.",
            Self::UnrealCache => "Unreal Engine cache. Unreal regenerates Intermediate and DerivedDataCache, but Saved also holds autosaves, logs and config — check it first.",
            Self::GodotCache => "Godot import cache. Godot reimports the assets on the next open.",
        }
    }

    /// Kinds that need no marker next to them; they belong to the nearest project above.
    fn searches_project(self) -> bool {
        matches!(
            self,
            Self::PythonCache | Self::PythonVenv | Self::RubyBundle
        )
    }
}

const GRADLE_MARKERS: &[&str] = &[
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
];
const PYTHON_MARKERS: &[&str] = &["pyproject.toml", "setup.py", "setup.cfg"];
const DOTNET_EXTENSIONS: &[&str] = &["csproj", "fsproj", "vbproj", "sln"];
const UNITY_DIRS: &[&str] = &["Library", "Temp", "Obj", "Logs"];
/// Files that identify a project root, used to group artifacts without a required marker
/// (`__pycache__` deep inside a package belongs to the project above it).
const PROJECT_MARKERS: &[&str] = &[
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
    "Pipfile",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
    "composer.json",
    "Gemfile",
    "pubspec.yaml",
    "Podfile",
    "Package.swift",
    "CMakeLists.txt",
    "mix.exs",
    "stack.yaml",
    "cabal.project",
    "build.sbt",
    "project.clj",
    "deps.edn",
    "build.zig",
    "project.godot",
    "go.mod",
];
const PROJECT_EXTENSIONS: &[&str] = &["csproj", "fsproj", "vbproj", "sln", "cabal", "uproject"];

/// Whether directory `dir` has a direct child file named `name`.
fn has_file(tree: &ScanTree, dir: NodeId, name: &str) -> bool {
    tree.children(dir)
        .any(|c| tree.name(c) == name && is_live_file(tree, c))
}

fn has_any_file(tree: &ScanTree, dir: NodeId, names: &[&str]) -> bool {
    tree.children(dir)
        .any(|c| is_live_file(tree, c) && names.contains(&tree.name(c)))
}

fn has_file_with_extension(tree: &ScanTree, dir: NodeId, exts: &[&str]) -> bool {
    tree.children(dir)
        .any(|c| is_live_file(tree, c) && exts.contains(&tree.extension(c)))
}

/// Whether directory `dir` has a direct child folder named `name`.
fn has_dir(tree: &ScanTree, dir: NodeId, name: &str) -> bool {
    tree.children(dir)
        .any(|c| tree.name(c) == name && tree.get(c).is_some_and(|n| n.is_dir() && !n.is_trashed()))
}

fn is_live_file(tree: &ScanTree, id: NodeId) -> bool {
    tree.get(id).is_some_and(|n| !n.is_dir() && !n.is_trashed())
}

fn is_project_root(tree: &ScanTree, dir: NodeId) -> bool {
    has_any_file(tree, dir, PROJECT_MARKERS)
        || has_file_with_extension(tree, dir, PROJECT_EXTENSIONS)
}

fn parent_of(tree: &ScanTree, id: NodeId) -> NodeId {
    tree.get(id).map_or(NO_NODE, |n| n.parent)
}

/// Artifact kind of directory `id` (whose parent is `parent`), checking the marker rules.
/// Arm order resolves names shared by several ecosystems (`build`, `target`, `obj`, `vendor`).
fn classify(tree: &ScanTree, id: NodeId, parent: NodeId) -> Option<JunkKind> {
    use JunkKind::*;
    let file = |name: &str| has_file(tree, parent, name);
    let any = |names: &[&str]| has_any_file(tree, parent, names);
    let ext = |exts: &[&str]| has_file_with_extension(tree, parent, exts);
    let name = tree.name(id);
    let kind = match name {
        "node_modules" if file("package.json") => NodeModules,
        ".next" | ".nuxt" | ".svelte-kit" if file("package.json") => NextBuild,
        "dist" | "build" | "out" if file("package.json") => JsBuild,
        ".angular" | ".parcel-cache" | ".turbo" | ".output" | ".docusaurus" | ".expo"
        | "coverage" | "storybook-static"
            if file("package.json") =>
        {
            JsToolCache
        }
        "target" if file("Cargo.toml") => CargoTarget,
        "target" if file("pom.xml") => MavenTarget,
        "target" | ".bloop" | ".metals" | ".bsp" if file("build.sbt") => ScalaBuild,
        "target" if any(&["project.clj", "deps.edn"]) => ClojureBuild,
        ".cpcache" if file("deps.edn") => ClojureBuild,
        ".venv" | "venv" | "env" if has_file(tree, id, "pyvenv.cfg") => PythonVenv,
        "__pycache__" | ".pytest_cache" | ".mypy_cache" | ".ruff_cache" | ".ipynb_checkpoints" => {
            PythonCache
        }
        ".tox" | ".nox" | "build" | "dist" if any(PYTHON_MARKERS) => PythonBuild,
        _ if name.ends_with(".egg-info") && any(PYTHON_MARKERS) => PythonBuild,
        "build" | ".gradle" | ".cxx" | ".externalNativeBuild" if any(GRADLE_MARKERS) => GradleBuild,
        ".dart_tool" | "build" if file("pubspec.yaml") => DartBuild,
        "build" if file("CMakeLists.txt") => CmakeBuild,
        _ if name.starts_with("cmake-build-") && file("CMakeLists.txt") => CmakeBuild,
        // Before .NET: Unity generates `.csproj` files next to its own `Obj` folder.
        _ if UNITY_DIRS.iter().any(|d| d.eq_ignore_ascii_case(name))
            && has_dir(tree, parent, "Assets")
            && has_dir(tree, parent, "ProjectSettings") =>
        {
            UnityCache
        }
        "bin" | "obj" if ext(DOTNET_EXTENSIONS) => DotnetBuild,
        "vendor" if file("composer.json") => PhpVendor,
        ".bundle" if file("Gemfile") => RubyBundle,
        "bundle"
            if tree.name(parent) == "vendor"
                && has_file(tree, parent_of(tree, parent), "Gemfile") =>
        {
            RubyBundle
        }
        "Pods" if file("Podfile") => SwiftBuild,
        ".build" if file("Package.swift") => SwiftBuild,
        "_build" | "deps" if file("mix.exs") => ElixirBuild,
        ".stack-work" if file("stack.yaml") => HaskellBuild,
        "dist-newstyle" if file("cabal.project") || ext(&["cabal"]) => HaskellBuild,
        "zig-cache" | ".zig-cache" | "zig-out" if file("build.zig") => ZigBuild,
        ".terraform" if ext(&["tf"]) => TerraformCache,
        "Intermediate" | "DerivedDataCache" | "Saved" if ext(&["uproject"]) => UnrealCache,
        ".godot" if file("project.godot") => GodotCache,
        _ => return None,
    };
    Some(kind)
}

/// Folders that were never listed (links, other volumes) or are gone cannot be artifacts.
fn is_scannable_dir(node: &Node) -> bool {
    node.is_dir()
        && !node.is_trashed()
        && !node
            .flags
            .intersects(NodeFlags::REPARSE | NodeFlags::OTHER_VOLUME)
}

/// Marks the top-most artifacts in the tree and returns how many were found. Clears marks
/// from a previous run first, so it can be called again after the tree changed.
pub fn detect_junk(tree: &mut ScanTree) -> usize {
    let mut found = Vec::new();
    let mut stack = vec![ROOT];
    while let Some(dir) = stack.pop() {
        for child in tree.children(dir) {
            let Some(node) = tree.get(child) else {
                continue;
            };
            if !is_scannable_dir(node) {
                continue;
            }
            match classify(tree, child, dir) {
                // Do not look inside an artifact: nested ones count once, with the top-most.
                Some(kind) => found.push((child, kind)),
                None if is_installed_software(tree.name(child)) => {}
                None => stack.push(child),
            }
        }
    }
    for id in 0..tree.len() as NodeId {
        tree.set_junk(id, 0);
    }
    for &(id, kind) in &found {
        tree.set_junk(id, kind as u8);
    }
    // Mark every folder above an artifact (the treemap's "contains artifacts" color and the
    // shortcut for project age). Walks stop at a folder that is already marked.
    for &(id, _) in &found {
        let mut cur = parent_of(tree, id);
        while let Some(n) = tree.get(cur) {
            if n.junk & CONTAINS_JUNK != 0 {
                break;
            }
            let (junk, parent) = (n.junk | CONTAINS_JUNK, n.parent);
            tree.set_junk(cur, junk);
            cur = parent;
        }
    }
    found.len()
}

/// Whether an artifact lies somewhere below folder `id` (as of the last [`detect_junk`]).
pub fn contains_junk(tree: &ScanTree, id: NodeId) -> bool {
    tree.get(id).is_some_and(|n| n.junk & CONTAINS_JUNK != 0)
}

/// One project with rebuildable artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JunkProject {
    /// Project folder (the one holding the marker file).
    pub dir: NodeId,
    /// Top-most artifacts of this project, largest first.
    pub artifacts: Vec<NodeId>,
    /// Totals of all artifacts (what cleaning would move to Trash).
    pub logical_size: u64,
    pub allocated_size: u64,
    pub file_count: u32,
    /// Newest file in the project outside any artifact (UNIX seconds, 0 if none) — the
    /// project's real "last worked on" time.
    pub newest_mtime: u32,
}

/// Groups artifacts marked by [`detect_junk`] by project, largest total first.
pub fn junk_projects(tree: &ScanTree) -> Vec<JunkProject> {
    let artifacts: Vec<NodeId> = (0..tree.len() as NodeId)
        .filter(|&id| {
            tree.get(id)
                .is_some_and(|n| n.junk & KIND_MASK != 0 && !n.is_trashed())
                && !tree
                    .ancestors(id)
                    .any(|a| tree.get(a).is_some_and(Node::is_trashed))
        })
        .collect();

    // Every folder that has an artifact somewhere below it: only those need a deep walk
    // when computing project age; others contribute their aggregate `newest_mtime`.
    let mut above_junk: HashSet<NodeId> = HashSet::new();
    for &id in &artifacts {
        for a in tree.ancestors(id) {
            if !above_junk.insert(a) {
                break;
            }
        }
    }

    let mut groups: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for &id in &artifacts {
        groups.entry(project_of(tree, id)).or_default().push(id);
    }

    let mut projects: Vec<JunkProject> = groups
        .into_iter()
        .map(|(dir, mut items)| {
            let contribution = |id: NodeId| tree.get(id).map_or((0, 0, 0), Node::contribution);
            items.sort_by_key(|&id| std::cmp::Reverse(contribution(id).1));
            let (logical_size, allocated_size, file_count) =
                items.iter().fold((0u64, 0u64, 0u32), |acc, &id| {
                    let (l, a, f) = contribution(id);
                    (acc.0 + l, acc.1 + a, acc.2.saturating_add(f))
                });
            JunkProject {
                dir,
                newest_mtime: newest_outside_junk(tree, dir, &above_junk),
                artifacts: items,
                logical_size,
                allocated_size,
                file_count,
            }
        })
        .collect();
    projects.sort_by(|a, b| {
        b.allocated_size
            .cmp(&a.allocated_size)
            .then(a.dir.cmp(&b.dir))
    });
    projects
}

/// Project folder of artifact `id`: its parent, except for kinds that need no marker next to
/// them (Python caches and virtualenvs, Ruby bundles), which belong to the nearest ancestor
/// that is a project root.
fn project_of(tree: &ScanTree, id: NodeId) -> NodeId {
    let parent = tree.get(id).map_or(ROOT, |n| n.parent);
    let needs_search = JunkKind::of(tree, id).is_some_and(JunkKind::searches_project);
    if !needs_search || parent == NO_NODE {
        return parent;
    }
    std::iter::once(parent)
        .chain(tree.ancestors(parent))
        .find(|&d| is_project_root(tree, d))
        .unwrap_or(parent)
}

/// Newest file mtime under `dir`, skipping artifact subtrees.
fn newest_outside_junk(tree: &ScanTree, dir: NodeId, above_junk: &HashSet<NodeId>) -> u32 {
    let mut newest = 0;
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for c in tree.children(d) {
            let Some(n) = tree.get(c) else { continue };
            if n.is_trashed() || n.junk & KIND_MASK != 0 {
                continue;
            }
            if n.is_dir() && above_junk.contains(&c) {
                stack.push(c);
            } else {
                newest = newest.max(n.newest_mtime);
            }
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scan, CancelToken, ScanOptions};
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    /// Creates files (`a/b.txt`) and folders (`a/b/`) under `root`.
    fn layout(root: &Path, entries: &[&str]) {
        for e in entries {
            let p = root.join(e);
            if e.ends_with('/') {
                fs::create_dir_all(&p).unwrap();
            } else {
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(&p, vec![0u8; 2048]).unwrap();
            }
        }
    }

    fn scanned(dir: &TempDir) -> ScanTree {
        let mut tree = scan(
            dir.path(),
            &ScanOptions::default(),
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        detect_junk(&mut tree);
        tree
    }

    fn id_of(tree: &ScanTree, rel: &str) -> NodeId {
        rel.split('/')
            .try_fold(ROOT, |id, name| tree.child_by_name(id, name))
            .unwrap_or_else(|| panic!("{rel} not in tree"))
    }

    fn kind(tree: &ScanTree, rel: &str) -> Option<JunkKind> {
        JunkKind::of(tree, id_of(tree, rel))
    }

    fn set_mtime(path: &Path, days_ago: u64) {
        let t = SystemTime::now() - Duration::from_secs(days_ago * 86_400);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn every_rule_matches_with_its_marker() {
        let dir = TempDir::new().unwrap();
        layout(
            dir.path(),
            &[
                "web/package.json",
                "web/node_modules/react/index.js",
                "web/.next/cache.bin",
                "web/dist/app.js",
                "rs/Cargo.toml",
                "rs/target/debug/app",
                "py/.venv/pyvenv.cfg",
                "py/src/__pycache__/m.pyc",
                "py/.pytest_cache/v",
                "java/pom.xml",
                "java/target/app.jar",
                "kt/build.gradle.kts",
                "kt/build/out.class",
                "kt/.gradle/cache",
                "cs/App.csproj",
                "cs/bin/App.dll",
                "cs/obj/App.obj",
            ],
        );
        let t = scanned(&dir);
        let expected = [
            ("web/node_modules", JunkKind::NodeModules),
            ("web/.next", JunkKind::NextBuild),
            ("web/dist", JunkKind::JsBuild),
            ("rs/target", JunkKind::CargoTarget),
            ("py/.venv", JunkKind::PythonVenv),
            ("py/src/__pycache__", JunkKind::PythonCache),
            ("py/.pytest_cache", JunkKind::PythonCache),
            ("java/target", JunkKind::MavenTarget),
            ("kt/build", JunkKind::GradleBuild),
            ("kt/.gradle", JunkKind::GradleBuild),
            ("cs/bin", JunkKind::DotnetBuild),
            ("cs/obj", JunkKind::DotnetBuild),
        ];
        for (path, k) in expected {
            assert_eq!(kind(&t, path), Some(k), "{path}");
        }
    }

    #[test]
    fn folders_without_a_marker_are_not_junk() {
        let dir = TempDir::new().unwrap();
        layout(
            dir.path(),
            &[
                "photos/build/img.jpg",
                "notes/dist/readme.txt",
                "game/bin/game.exe",
                "game/obj/model.obj",
                "rust-docs/target/index.html",
                "home/env/config.ini",
                "deps/node_modules/x.js",
                // Marker in the folder itself, not next to it.
                "nested/build/package.json",
            ],
        );
        let t = scanned(&dir);
        for path in [
            "photos/build",
            "notes/dist",
            "game/bin",
            "game/obj",
            "rust-docs/target",
            "home/env",
            "deps/node_modules",
            "nested/build",
        ] {
            assert_eq!(kind(&t, path), None, "{path}");
        }
        assert!(junk_projects(&t).is_empty());
    }

    #[test]
    fn nested_artifacts_count_once_with_the_top_most() {
        let dir = TempDir::new().unwrap();
        layout(
            dir.path(),
            &[
                "app/package.json",
                "app/node_modules/lib/package.json",
                "app/node_modules/lib/node_modules/dep/index.js",
                "app/node_modules/lib/dist/lib.js",
            ],
        );
        let t = scanned(&dir);
        assert_eq!(kind(&t, "app/node_modules"), Some(JunkKind::NodeModules));
        assert_eq!(kind(&t, "app/node_modules/lib/node_modules"), None);
        assert_eq!(kind(&t, "app/node_modules/lib/dist"), None);

        let projects = junk_projects(&t);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].artifacts, vec![id_of(&t, "app/node_modules")]);
        assert_eq!(projects[0].file_count, 3);
    }

    #[test]
    fn artifacts_are_grouped_by_project_with_totals() {
        let dir = TempDir::new().unwrap();
        layout(
            dir.path(),
            &[
                "big/package.json",
                "big/node_modules/a.js",
                "big/node_modules/b.js",
                "big/node_modules/c.js",
                "big/dist/app.js",
                "small/Cargo.toml",
                "small/target/app",
                "pyproj/pyproject.toml",
                "pyproj/pkg/sub/__pycache__/m.pyc",
            ],
        );
        let t = scanned(&dir);
        let projects = junk_projects(&t);
        let dirs: Vec<NodeId> = projects.iter().map(|p| p.dir).collect();
        assert_eq!(dirs[0], id_of(&t, "big"), "largest project first");
        let big = &projects[0];
        assert_eq!(
            big.artifacts,
            vec![id_of(&t, "big/node_modules"), id_of(&t, "big/dist")],
            "largest artifact first"
        );
        assert_eq!((big.file_count, big.logical_size), (4, 4 * 2048));
        // `__pycache__` deep in a package belongs to the folder with `pyproject.toml`.
        assert!(dirs.contains(&id_of(&t, "pyproj")));
        assert!(dirs.contains(&id_of(&t, "small")));
        assert_eq!(projects.len(), 3);
    }

    #[test]
    fn project_age_ignores_files_inside_artifacts() {
        let dir = TempDir::new().unwrap();
        layout(
            dir.path(),
            &[
                "old/package.json",
                "old/src/index.js",
                "old/node_modules/fresh.js",
            ],
        );
        set_mtime(&dir.path().join("old/package.json"), 400);
        set_mtime(&dir.path().join("old/src/index.js"), 365);
        // A fresh `npm install` must not make the project look active.
        let t = scanned(&dir);
        let project = &junk_projects(&t)[0];
        let index_mtime = t.node(id_of(&t, "old/src/index.js")).unwrap().newest_mtime;
        assert_eq!(project.newest_mtime, index_mtime);
        assert!(t.node(id_of(&t, "old")).unwrap().newest_mtime > index_mtime);
    }

    #[test]
    fn trashed_artifacts_drop_out_and_detection_can_rerun() {
        let dir = TempDir::new().unwrap();
        layout(
            dir.path(),
            &["a/Cargo.toml", "a/target/x", "b/package.json", "b/dist/y"],
        );
        let mut t = scanned(&dir);
        t.mark_trashed(id_of(&t, "a/target")).unwrap();
        let projects = junk_projects(&t);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].dir, id_of(&t, "b"));
        assert_eq!(
            detect_junk(&mut t),
            1,
            "trashed artifacts are not re-detected"
        );
        assert_eq!(kind(&t, "a/target"), None);
    }

    #[test]
    fn extended_rules_match_with_their_markers() {
        let dir = TempDir::new().unwrap();
        layout(
            dir.path(),
            &[
                "ng/package.json",
                "ng/.angular/cache/x",
                "ng/.turbo/log",
                "ng/coverage/lcov.info",
                "lib/pyproject.toml",
                "lib/.tox/py312/x",
                "lib/build/lib/m.py",
                "lib/pkg.egg-info/PKG-INFO",
                "lib/.ruff_cache/x",
                "web/composer.json",
                "web/vendor/autoload.php",
                "rails/Gemfile",
                "rails/.bundle/config",
                "rails/vendor/bundle/ruby/gem.rb",
                "flutter/pubspec.yaml",
                "flutter/.dart_tool/x",
                "flutter/build/app.apk",
                "ios/Podfile",
                "ios/Pods/Lib/x.m",
                "swift/Package.swift",
                "swift/.build/debug/x",
                "android/build.gradle",
                "android/.cxx/x.o",
                "cpp/CMakeLists.txt",
                "cpp/build/a.o",
                "cpp/cmake-build-debug/b.o",
                "ex/mix.exs",
                "ex/_build/dev/x",
                "ex/deps/plug/x",
                "hs/stack.yaml",
                "hs/.stack-work/x",
                "cabal/app.cabal",
                "cabal/dist-newstyle/x",
                "sc/build.sbt",
                "sc/target/x.class",
                "sc/.metals/x",
                "clj/deps.edn",
                "clj/.cpcache/x",
                "clj/target/x.jar",
                "zig/build.zig",
                "zig/.zig-cache/x",
                "zig/zig-out/bin/x",
                "tf/main.tf",
                "tf/.terraform/providers/x",
                "unity/Assets/",
                "unity/ProjectSettings/",
                "unity/Game.csproj",
                "unity/Library/x",
                "unity/Temp/x",
                "unity/obj/x",
                "ue/Game.uproject",
                "ue/Intermediate/x",
                "ue/DerivedDataCache/x",
                "gd/project.godot",
                "gd/.godot/x",
            ],
        );
        let t = scanned(&dir);
        use JunkKind::*;
        let expected = [
            ("ng/.angular", JsToolCache),
            ("ng/.turbo", JsToolCache),
            ("ng/coverage", JsToolCache),
            ("lib/.tox", PythonBuild),
            ("lib/build", PythonBuild),
            ("lib/pkg.egg-info", PythonBuild),
            ("lib/.ruff_cache", PythonCache),
            ("web/vendor", PhpVendor),
            ("rails/.bundle", RubyBundle),
            ("rails/vendor/bundle", RubyBundle),
            ("flutter/.dart_tool", DartBuild),
            ("flutter/build", DartBuild),
            ("ios/Pods", SwiftBuild),
            ("swift/.build", SwiftBuild),
            ("android/.cxx", GradleBuild),
            ("cpp/build", CmakeBuild),
            ("cpp/cmake-build-debug", CmakeBuild),
            ("ex/_build", ElixirBuild),
            ("ex/deps", ElixirBuild),
            ("hs/.stack-work", HaskellBuild),
            ("cabal/dist-newstyle", HaskellBuild),
            ("sc/target", ScalaBuild),
            ("sc/.metals", ScalaBuild),
            ("clj/.cpcache", ClojureBuild),
            ("clj/target", ClojureBuild),
            ("zig/.zig-cache", ZigBuild),
            ("zig/zig-out", ZigBuild),
            ("tf/.terraform", TerraformCache),
            ("unity/Library", UnityCache),
            ("unity/Temp", UnityCache),
            // Unity's generated `.csproj` must not turn its `obj` into a .NET artifact.
            ("unity/obj", UnityCache),
            ("ue/Intermediate", UnrealCache),
            ("ue/DerivedDataCache", UnrealCache),
            ("gd/.godot", GodotCache),
        ];
        for (path, k) in expected {
            assert_eq!(kind(&t, path), Some(k), "{path}");
        }
        assert_eq!(
            kind(&t, "rails/vendor"),
            None,
            "only the bundle inside vendor"
        );

        // `vendor/bundle` belongs to the Rails app, not to `vendor`.
        let rails = junk_projects(&t)
            .into_iter()
            .find(|p| p.dir == id_of(&t, "rails"))
            .unwrap();
        assert_eq!(rails.artifacts.len(), 2);
        assert!(!junk_projects(&t)
            .iter()
            .any(|p| p.dir == id_of(&t, "rails/vendor")));
    }

    #[test]
    fn extended_rules_need_their_markers() {
        let dir = TempDir::new().unwrap();
        layout(
            dir.path(),
            &[
                "docs/coverage/report.html",
                "docs/.output/x",
                "go/go.mod",
                "go/vendor/lib.go",
                "site/vendor/bundle/x",
                "erl/deps/x",
                "erl/_build/x",
                "mac/Pods/x",
                "mac/.build/x",
                "c/cmake-build-debug/x",
                "c/build/x",
                "ops/.terraform/x",
                "ops/main.tf.bak",
                "books/Library/x",
                "books/Temp/x",
                "half/Assets/",
                "half/Library/x",
                "film/Intermediate/x",
                "film/Saved/x",
                "sbt/target/x",
                "sbt/.metals/x",
                "old/x.egg-info/PKG-INFO",
                "old/.tox/x",
            ],
        );
        let t = scanned(&dir);
        for path in [
            "docs/coverage",
            "docs/.output",
            "go/vendor",
            "site/vendor/bundle",
            "erl/deps",
            "erl/_build",
            "mac/Pods",
            "mac/.build",
            "c/cmake-build-debug",
            "c/build",
            "ops/.terraform",
            "books/Library",
            "books/Temp",
            "half/Library",
            "film/Intermediate",
            "film/Saved",
            "sbt/target",
            "sbt/.metals",
            "old/x.egg-info",
            "old/.tox",
        ] {
            assert_eq!(kind(&t, path), None, "{path}");
        }
        assert!(junk_projects(&t).is_empty());
    }

    #[test]
    fn installed_software_is_never_searched() {
        let dir = TempDir::new().unwrap();
        layout(
            dir.path(),
            &[
                // VS Code extension: `dist/` next to `package.json` is the extension itself.
                ".vscode/extensions/acme.tool-1.0.0/package.json",
                ".vscode/extensions/acme.tool-1.0.0/dist/extension.js",
                // Global npm package and an Electron app under AppData.
                "AppData/Roaming/npm/node_modules/pnpm/package.json",
                "AppData/Roaming/npm/node_modules/pnpm/dist/pnpm.cjs",
                "AppData/Local/Programs/Editor/resources/app/package.json",
                "AppData/Local/Programs/Editor/resources/app/node_modules/x.js",
                // Installed Python package and a macOS app bundle.
                "py/lib/site-packages/pkg/setup.py",
                "py/lib/site-packages/pkg/build/x",
                "Tools.app/Contents/Resources/app/package.json",
                "Tools.app/Contents/Resources/app/out/main.js",
                // Repository internals.
                "repo/.git/modules/sub/package.json",
                "repo/.git/modules/sub/dist/x",
                // A real project still works.
                "real/package.json",
                "real/dist/app.js",
            ],
        );
        let t = scanned(&dir);
        let projects = junk_projects(&t);
        assert_eq!(projects.len(), 1, "{projects:?}");
        assert_eq!(projects[0].artifacts, vec![id_of(&t, "real/dist")]);
    }

    #[test]
    fn folders_above_artifacts_are_marked() {
        let dir = TempDir::new().unwrap();
        layout(
            dir.path(),
            &[
                "code/app/package.json",
                "code/app/dist/a.js",
                "docs/readme.md",
            ],
        );
        let mut t = scanned(&dir);
        assert!(contains_junk(&t, ROOT));
        assert!(contains_junk(&t, id_of(&t, "code")));
        assert!(contains_junk(&t, id_of(&t, "code/app")));
        assert!(
            !contains_junk(&t, id_of(&t, "code/app/dist")),
            "the artifact itself"
        );
        assert!(!contains_junk(&t, id_of(&t, "docs")));
        // The mark does not leak into the kind.
        assert_eq!(JunkKind::of(&t, id_of(&t, "code/app")), None);
        assert_eq!(kind(&t, "code/app/dist"), Some(JunkKind::JsBuild));

        t.mark_trashed(id_of(&t, "code/app/dist")).unwrap();
        detect_junk(&mut t);
        assert!(!contains_junk(&t, id_of(&t, "code")));
        assert!(!contains_junk(&t, ROOT));
    }

    #[test]
    fn kind_round_trips_through_u8_and_has_stable_names() {
        for (i, &k) in JunkKind::ALL.iter().enumerate() {
            assert_eq!(k as usize, i + 1, "codes are contiguous and append-only");
            assert_eq!(JunkKind::from_u8(k as u8), Some(k));
        }
        assert_eq!(JunkKind::from_u8(0), None);
        assert_eq!(JunkKind::CargoTarget.as_str(), "cargo_target");
        let names: HashSet<_> = JunkKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(names.len(), JunkKind::ALL.len());
    }
}
