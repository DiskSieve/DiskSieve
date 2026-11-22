// Typed wrappers around Tauri `invoke()` with a browser fallback.
//
// Inside the desktop app every call goes to Rust. In a plain browser (`npm run dev`) there is
// no Tauri runtime, so the same functions resolve with demo data — the UI code never branches.
import { Channel, invoke } from "@tauri-apps/api/core";
import type { AdviceDto, AppInfoDto, ChangeDto, ChangeKindDto, ChangesDto, DevJunkDto, DriveDto, DuplicateGroupDto, DuplicateProgressDto, DuplicatesDto, JunkProjectDto, ScanProgressDto, ScanSummaryDto, SearchResultDto, SieveDto, SieveItemDto, SizeMetricDto, TrashReportDto, TreemapNodeDto, ViewportDto } from "@/types/ipc";
import { DRIVE_CAPACITY, MOCK_DUPLICATES, TOTAL_FILES, findPath, flatten, initialDrive, type FsNode, type JunkTag } from "@/data/mockDrive";
import { CONTAINS_JUNK_FLAG } from "@/lib/treemap-palette";
import { version as appVersion } from "../../package.json";

/** True when running inside the Tauri desktop shell (not a plain browser tab). */
export const isTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Error raised when a Rust command fails; `message` is the error string returned by Rust. */
export class IpcError extends Error {
  constructor(readonly command: string, message: string) {
    super(message);
    this.name = "IpcError";
  }
}

type NoArgs = Record<string, never>;

// Command name → argument and result types. Names must match `#[tauri::command]` fns
// registered in `src-tauri/src/lib.rs`; result types come from the generated DTOs.
interface Commands {
  app_info: { args: NoArgs; result: AppInfoDto };
  list_drives: { args: NoArgs; result: DriveDto[] };
  pick_folder: { args: NoArgs; result: string | null };
  start_scan: { args: { path: string; onProgress: Channel<ScanProgressDto> }; result: ScanSummaryDto | null };
  cancel_scan: { args: NoArgs; result: boolean };
  get_viewport: { args: { generation: number; nodeId: number | null; metric: SizeMetricDto; delta: boolean }; result: ViewportDto };
  get_dev_junk: { args: { generation: number }; result: DevJunkDto };
  get_changes: { args: { generation: number; kind: ChangeKindDto | null; limit: number | null }; result: ChangesDto };
  find_duplicates: { args: { generation: number; onProgress: Channel<DuplicateProgressDto> }; result: DuplicatesDto | null };
  get_duplicates: { args: { generation: number }; result: DuplicatesDto | null };
  cancel_duplicates: { args: NoArgs; result: boolean };
  get_advice: { args: { generation: number; nodeId: number }; result: AdviceDto };
  review_sieve: { args: { generation: number; nodeIds: number[] }; result: SieveDto };
  search_nodes: { args: { generation: number; query: string; metric: SizeMetricDto; limit: number | null }; result: SearchResultDto };
  trash_nodes: { args: { generation: number; nodeIds: number[] }; result: TrashReportDto };
  restore_nodes: { args: { generation: number; nodeIds: number[] }; result: TrashReportDto };
  reveal_in_explorer: { args: { generation: number; nodeId: number }; result: null };
  open_trash: { args: NoArgs; result: null };
}

/** Maximum number of individual nodes in a viewport (mirrors `DEFAULT_MAX_ITEMS` in Rust). */
export const MAX_VIEWPORT_ITEMS = 200;
/** Hits returned by `search_nodes` (Top-50 by size). */
export const SEARCH_LIMIT = 50;

async function call<K extends keyof Commands>(
  command: K,
  args: Commands[K]["args"],
): Promise<Commands[K]["result"]> {
  try {
    return await invoke<Commands[K]["result"]>(command, args);
  } catch (e) {
    throw new IpcError(command, typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
  }
}

// ── Browser fallback (demo data) ───────────────────────────────────────────────
const MOCK_SCAN_MS = 2500;
const MOCK_THREADS = typeof navigator !== "undefined" ? navigator.hardwareConcurrency || 8 : 8;
let mockGeneration = 0;
let cancelMockScan: (() => void) | null = null;
let mockTree: FsNode = initialDrive;
/** Demo "Unscanned / hidden" space on C: (shadow copies, protected folders, metadata). */
const MOCK_HIDDEN_BYTES = 18.4 * 1024 ** 3;
const MOCK_SKIPPED = ["C:\\System Volume Information", "C:\\Windows\\CSC", "C:\\Users\\Default User"];
const MOCK_DIRS = ["Windows\\System32", "Users\\dev\\AppData\\Local", "Program Files", "Users\\dev\\Projects", "Games\\Steam\\steamapps", "Users\\dev\\Downloads"];

/** Lets the demo's in-memory "trash" change the tree the mock viewport is built from. */
export const setMockTree = (tree: FsNode) => {
  mockTree = tree;
};

/** Numeric id of a demo node (`"n42"` → 42). */
export const mockNodeId = (n: FsNode) => Number(n.id.slice(1));
/** Demo tags → real `JunkKind` ids. Docker is a global tool cache (step 7.5), not project junk. */
const MOCK_KIND: Record<Exclude<JunkTag, null>, string | null> = {
  node_modules: "node_modules", cargo_target: "cargo_target", python_venv: "python_venv", gradle: "gradle_build", docker_cache: null,
};
const mockKind = (n: FsNode) => (n.junkTag ? MOCK_KIND[n.junkTag] : null);
const containsJunk = (n: FsNode): boolean => !!mockKind(n) || !!n.children?.some(containsJunk);

function mockNodeDto(n: FsNode, metric: SizeMetricDto, nowSecs: number): TreemapNodeDto {
  const allocatedSize = n.sizeBytes;
  const logicalSize = Math.round(n.sizeBytes * 0.985);
  return {
    id: mockNodeId(n),
    name: n.name,
    path: n.path,
    value: metric === "allocated" ? allocatedSize : logicalSize,
    logicalSize,
    allocatedSize,
    deltaBytes: n.deltaBytes - removedBelow(n),
    isDir: n.isDir,
    fileCount: n.fileCount,
    newestMtime: nowSecs - n.lastModifiedDaysAgo * 86_400,
    extension: n.isDir || !n.extension ? null : n.extension,
    junkKind: mockKind(n),
    flags: !mockKind(n) && containsJunk(n) ? [CONTAINS_JUNK_FLAG] : [],
    otherCount: null,
  };
}

/** Demo items "removed since the previous scan", keyed by the folder they were in. */
const MOCK_REMOVED: Record<string, { name: string; allocatedSize: number; isDir: boolean }[]> = {
  Downloads: [
    { name: "ubuntu-24.04-desktop-amd64.iso", allocatedSize: 6_114_656_256, isDir: false },
    { name: "old-installers", allocatedSize: 2_351_000_000, isDir: true },
  ],
  Videos: [{ name: "obs-recording-2026-10-02.mkv", allocatedSize: 3_870_000_000, isDir: false }],
};
const removedHere = (n: FsNode) => (n.isDir ? MOCK_REMOVED[n.name] ?? [] : []);
const removedBelow = (n: FsNode): number =>
  removedHere(n).reduce((a, r) => a + r.allocatedSize, 0) + (n.children ?? []).reduce((a, c) => a + removedBelow(c), 0);

function mockOther(nodes: TreemapNodeDto[], path: string, count: number): TreemapNodeDto {
  const sum = (f: (n: TreemapNodeDto) => number) => nodes.reduce((a, n) => a + f(n), 0);
  return {
    id: null, name: "Other", path,
    value: sum((n) => n.value), logicalSize: sum((n) => n.logicalSize), allocatedSize: sum((n) => n.allocatedSize),
    deltaBytes: sum((n) => n.deltaBytes ?? 0), isDir: false, fileCount: sum((n) => n.fileCount),
    newestMtime: Math.max(0, ...nodes.map((n) => n.newestMtime)), extension: null, junkKind: null, flags: [], otherCount: count,
  };
}

/** Demo copies of a few Rust Safety Advisor rules, so the Inspector shows real-looking verdicts. */
const MOCK_RULES: Record<string, Omit<AdviceDto, "generation" | "nodeId">> = {
  "hiberfil.sys": {
    level: "system", ruleId: "hiberfil", title: "Hibernation file",
    description: "Windows saves the memory contents here for hibernation and Fast Startup. Deleted by hand, it comes back at the next restart.",
    alternative: { how: "Turn hibernation off (this also disables Fast Startup) in a terminal run as administrator.", command: "powercfg /hibernate off" },
    protectedReason: "System file — managed by the operating system.",
  },
  "pagefile.sys": {
    level: "system", ruleId: "pagefile", title: "Page file",
    description: "Windows virtual memory, in use all the time. Its size is managed by Windows.",
    alternative: { how: "Resize or move it in System Properties: Advanced → Performance → Settings → Advanced → Virtual memory.", command: "SystemPropertiesPerformance.exe" },
    protectedReason: "System file — managed by the operating system.",
  },
  shadercache: {
    level: "safe", ruleId: "steam_shader_cache", title: "Steam shader cache",
    description: "Precompiled shaders. Games and the graphics driver rebuild them on the next launch; the first start of each game may stutter for a moment.",
    alternative: null, protectedReason: null,
  },
};

/** Demo verdict: a mirrored rule, else derived from the demo hint text. */
function mockAdvice(generation: number, nodeId: number): AdviceDto {
  const n = flatten(mockTree).find((x) => mockNodeId(x) === nodeId);
  if (!n) throw new Error(`node ${nodeId} does not exist in the scan tree`);
  const rule = MOCK_RULES[n.name.toLowerCase()];
  if (rule) return { generation, nodeId, ...rule };
  const hint = n.safetyHint;
  const system = hint.startsWith("SYSTEM:");
  const safe = hint.startsWith("Safe");
  return {
    generation, nodeId,
    level: system ? "system" : safe ? "safe" : "review",
    ruleId: mockKind(n) ? "dev_junk" : null,
    title: system ? "Protected by DiskSieve" : mockKind(n) ? "Rebuildable build artifact" : safe ? "Recreated automatically" : "No rule for this item",
    description: hint.replace(/^(SYSTEM:|Safe to delete —)\s*/, "").replace(/^./, (c) => c.toUpperCase()),
    alternative: null,
    protectedReason: system ? "System location — removing it could break the operating system or installed apps." : null,
  };
}

/** Demo Sieve Basket review: same rules as `review_sieve` in Rust (nesting by path). */
function mockReviewSieve(generation: number, nodeIds: number[]): SieveDto {
  const byId = new Map(flatten(mockTree).map((n) => [mockNodeId(n), n]));
  const ids = [...new Set(nodeIds)];
  const missing = ids.filter((id) => !byId.has(id));
  const judged = ids.filter((id) => byId.has(id)).map((id) => ({ n: byId.get(id)!, advice: mockAdvice(generation, id) }));
  const movable = judged.filter((j) => j.advice.level !== "system" && j.n !== mockTree);
  const nowSecs = Math.floor(Date.now() / 1000);
  const dto: SieveDto = {
    generation, items: [], missing, allocatedBytes: 0, logicalBytes: 0, fileCount: 0,
    safeCount: 0, reviewCount: 0, protectedCount: 0, trashableIds: [],
  };
  for (const { n, advice } of judged) {
    const root = n === mockTree;
    if (root) Object.assign(advice, { level: "system", title: "Scan root", protectedReason: "The scanned folder itself cannot be moved to Trash." });
    // Closest collected folder above (the one whose path is longest).
    const cover = movable
      .filter((m) => m.n !== n && n.path.startsWith(m.n.path.replace(/\\?$/, "\\")))
      .sort((a, b) => b.n.path.length - a.n.path.length)[0];
    const status: SieveItemDto["status"] = cover ? "covered" : advice.level === "system" ? "protected" : "ready";
    const node = mockNodeDto(n, "allocated", nowSecs);
    dto.items.push({ node, status, coveredBy: cover ? mockNodeId(cover.n) : null, advice });
    if (status === "protected") dto.protectedCount++;
    if (status !== "ready") continue;
    dto.allocatedBytes += node.allocatedSize;
    dto.logicalBytes += node.logicalSize;
    dto.fileCount += node.fileCount;
    if (advice.level === "safe") dto.safeCount++;
    else dto.reviewCount++;
    dto.trashableIds.push(mockNodeId(n));
  }
  return dto;
}

const mock = {
  appInfo: (): AppInfoDto => ({
    name: "DiskSieve",
    version: appVersion,
    coreVersion: appVersion,
    os: "browser",
    arch: "demo",
  }),

  drives: (): DriveDto[] => [
    { mountPoint: "C:\\", name: "Local Disk", fileSystem: "NTFS", kind: "ssd", totalBytes: DRIVE_CAPACITY, availableBytes: DRIVE_CAPACITY - initialDrive.sizeBytes - MOCK_HIDDEN_BYTES, isRemovable: false },
    { mountPoint: "D:\\", name: "Archive", fileSystem: "NTFS", kind: "hdd", totalBytes: 2 * DRIVE_CAPACITY, availableBytes: 0.8 * DRIVE_CAPACITY, isRemovable: false },
  ],

  summary: (path: string, generation: number, durationMs: number): ScanSummaryDto => {
    const scannedBytes = initialDrive.sizeBytes;
    const drive = mock.drives().find((d) => d.mountPoint === path);
    const usedBytes = drive ? Math.max(0, drive.totalBytes - drive.availableBytes) : 0;
    return {
      generation,
      rootPath: path,
      fileCount: TOTAL_FILES,
      dirCount: Math.round(TOTAL_FILES / 9),
      logicalBytes: Math.round(scannedBytes * 0.97),
      allocatedBytes: scannedBytes,
      skippedEntries: MOCK_SKIPPED.length,
      skippedSamples: MOCK_SKIPPED,
      durationMs,
      volume: drive ? {
        totalBytes: drive.totalBytes,
        usedBytes,
        freeBytes: drive.availableBytes,
        scannedBytes,
        unaccountedBytes: Math.max(0, usedBytes - scannedBytes),
      } : null,
      // The demo pretends the same drive was scanned yesterday (growth data in mockDrive.ts).
      previousSnapshotAt: Math.floor(Date.now() / 1000) - 86_400,
      snapshotWarning: null,
    };
  },

  /** Simulated scan: progress every 100 ms for ~2.5 s, cancellable like the real one. */
  scan: (path: string, onProgress: (p: ScanProgressDto) => void): Promise<ScanSummaryDto | null> => {
    cancelMockScan?.();
    const generation = ++mockGeneration;
    return new Promise((resolve) => {
      const t0 = performance.now();
      let timer: ReturnType<typeof setTimeout> | undefined;
      cancelMockScan = () => {
        clearTimeout(timer);
        cancelMockScan = null;
        resolve(null);
      };
      const tick = () => {
        const f = Math.min(1, (performance.now() - t0) / MOCK_SCAN_MS);
        onProgress({
          generation,
          files: Math.round(TOTAL_FILES * f),
          dirs: Math.round((TOTAL_FILES / 9) * f),
          allocatedBytes: Math.round(initialDrive.sizeBytes * f),
          skippedEntries: f > 0.5 ? MOCK_SKIPPED.length : 0,
          currentDir: f < 1 ? `${path}${MOCK_DIRS[Math.floor(f * MOCK_DIRS.length) % MOCK_DIRS.length]}` : path,
          elapsedMs: Math.round(performance.now() - t0),
          threads: MOCK_THREADS,
        });
        if (f < 1) {
          timer = setTimeout(tick, 100);
        } else {
          cancelMockScan = null;
          resolve(mock.summary(path, generation, Math.round(performance.now() - t0)));
        }
      };
      tick();
    });
  },
};

/** Demo projection, same shape and rules as `build_viewport` in Rust. */
function mockViewport(generation: number, nodeId: number | null, metric: SizeMetricDto, delta: boolean): ViewportDto {
  const target = nodeId === null ? mockTree : flatten(mockTree).find((n) => mockNodeId(n) === nodeId);
  if (!target) throw new IpcError("get_viewport", `node ${nodeId} does not exist in the scan tree`);
  if (!target.isDir) throw new IpcError("get_viewport", `operation not allowed on node ${nodeId}: not a directory`);
  const nowSecs = Math.floor(Date.now() / 1000);
  const trail = findPath(mockTree, target.id) ?? [target];
  const change = (n: TreemapNodeDto) => n.deltaBytes ?? 0;
  const children = (target.children ?? []).map((c) => mockNodeDto(c, metric, nowSecs))
    .filter((n) => !delta || change(n) !== 0)
    // Same order as Rust: the biggest changes (either way) are kept, shown by growth.
    .sort(delta ? (a, b) => Math.abs(change(b)) - Math.abs(change(a)) : (a, b) => b.value - a.value);
  const nodes = children.slice(0, MAX_VIEWPORT_ITEMS);
  if (delta) nodes.sort((a, b) => change(b) - change(a));
  const rest = children.slice(MAX_VIEWPORT_ITEMS);
  if (rest.length) nodes.push(mockOther(rest, target.path, rest.length));
  // Demo folders without listed contents ("blobs") are shown as one aggregated node.
  if (!children.length && target.sizeBytes > 0 && (!delta || target.deltaBytes !== 0)) {
    nodes.push(mockOther([mockNodeDto(target, metric, nowSecs)], target.path, target.fileCount));
  }
  const removed = delta ? removedHere(target) : [];
  return {
    generation,
    currentId: mockNodeId(target),
    currentPath: target.path,
    breadcrumbs: trail.map((n) => ({ id: mockNodeId(n), name: n.name })),
    metric,
    totalValue: nodes.reduce((a, n) => a + n.value, 0),
    deltaBytes: target.deltaBytes - removedBelow(target),
    deltaMode: delta,
    nodes,
    removed: removed.map((r) => ({ ...r, path: `${target.path.replace(/\\$/, "")}\\${r.name}` })),
    removedCount: removed.length,
  };
}

/** Demo Changes view: leaf items of the demo tree with growth, plus the demo removed items. */
function mockChanges(generation: number, kind: ChangeKindDto | null, limit: number): ChangesDto {
  const all: ChangeDto[] = [];
  const walk = (n: FsNode, parent: FsNode) => {
    for (const r of removedHere(n)) {
      all.push({ kind: "removed", nodeId: null, folderId: mockNodeId(n), name: r.name, path: `${n.path.replace(/\\$/, "")}\\${r.name}`,
        isDir: r.isDir, bytes: -r.allocatedSize, beforeSize: r.allocatedSize, afterSize: null });
    }
    if (n.children?.length) {
      n.children.forEach((c) => walk(c, n));
      return;
    }
    if (n.deltaBytes === 0) return;
    // Demo sizes are rounded; a change of (almost) the whole size means the item is new.
    const added = n.deltaBytes >= n.sizeBytes * 0.999;
    all.push({ kind: added ? "added" : n.deltaBytes > 0 ? "grew" : "shrank", nodeId: mockNodeId(n), folderId: mockNodeId(parent), name: n.name, path: n.path,
      isDir: n.isDir, bytes: n.deltaBytes, beforeSize: added ? null : n.sizeBytes - n.deltaBytes, afterSize: n.sizeBytes });
  };
  walk(mockTree, mockTree);
  const kinds: ChangeKindDto[] = ["added", "removed", "grew", "shrank", "smallFiles"];
  const totals = kinds.map((k) => {
    const of = all.filter((c) => c.kind === k);
    return { kind: k, count: of.length, bytes: of.reduce((a, c) => a + c.bytes, 0) };
  });
  const grown = all.reduce((a, c) => a + Math.max(c.bytes, 0), 0);
  const shrunk = all.reduce((a, c) => a + Math.max(-c.bytes, 0), 0);
  const matching = all.filter((c) => kind === null || c.kind === kind).sort((a, b) => Math.abs(b.bytes) - Math.abs(a.bytes) || b.bytes - a.bytes);
  return {
    generation, previousScanAt: Math.floor(Date.now() / 1000) - 86_400, kind,
    changes: matching.slice(0, limit), matching: matching.length, totals, grown, shrunk, net: grown - shrunk,
  };
}

/** Demo duplicate search finished for this generation (results are rebuilt from `mockTree`). */
let mockDupSearch: { generation: number; durationMs: number } | null = null;
let cancelMockDups: (() => void) | null = null;
const MOCK_DUP_MS = 1800;
/** Files of a shared size the demo "checks" besides the real duplicates. */
const MOCK_DUP_EXTRA_FILES = 1342;

/** Demo duplicates: `MOCK_DUPLICATES` resolved against the current demo tree (trashed copies drop out). */
function mockDuplicates(generation: number): DuplicatesDto | null {
  if (!mockDupSearch || mockDupSearch.generation !== generation) return null;
  const nowSecs = Math.floor(Date.now() / 1000);
  const byPath = new Map<string, { n: FsNode; parent: FsNode }>();
  const walk = (n: FsNode) => n.children?.forEach((c) => { byPath.set(c.path, { n: c, parent: n }); walk(c); });
  walk(mockTree);
  const groups: DuplicateGroupDto[] = [];
  for (const g of MOCK_DUPLICATES) {
    const found = g.paths.flatMap((p) => byPath.get(p) ?? []).sort((a, b) => b.n.lastModifiedDaysAgo - a.n.lastModifiedDaysAgo);
    if (found.length < 2) continue;
    const size = found[0].n.sizeBytes;
    groups.push({
      hash: (g.hash + "e1d4c07b93a25f68").repeat(3).slice(0, 64), name: found[0].n.name, sizeBytes: size, wastedBytes: size * (found.length - 1),
      copies: found.map(({ n, parent }) => ({ nodeId: mockNodeId(n), folderId: mockNodeId(parent), path: n.path, mtime: nowSecs - n.lastModifiedDaysAgo * 86_400 })),
    });
  }
  groups.sort((a, b) => b.wastedBytes - a.wastedBytes);
  const copyCount = groups.reduce((a, g) => a + g.copies.length, 0);
  return {
    generation, groups, groupCount: groups.length, copyCount, wastedBytes: groups.reduce((a, g) => a + g.wastedBytes, 0),
    filesChecked: MOCK_DUP_EXTRA_FILES + copyCount, bytesRead: 61.3 * 1024 ** 3, skipped: 2, durationMs: mockDupSearch.durationMs,
  };
}

/** Demo duplicate search: a quick pass, then a full pass, with progress ~10× per second. */
function mockFindDuplicates(generation: number, onProgress: (p: DuplicateProgressDto) => void): Promise<DuplicatesDto | null> {
  cancelMockDups?.();
  return new Promise((resolve) => {
    const started = Date.now();
    const quickFiles = MOCK_DUP_EXTRA_FILES + 12;
    const fullFiles = 14;
    const quickBytes = quickFiles * 32 * 1024;
    const fullBytes = 42.6 * 1024 ** 3;
    const timer = setInterval(() => {
      const t = Math.min(1, (Date.now() - started) / MOCK_DUP_MS);
      const quick = t < 0.35;
      const f = quick ? t / 0.35 : (t - 0.35) / 0.65;
      onProgress({
        generation, phase: quick ? "quick" : "full", elapsedMs: Date.now() - started,
        filesDone: Math.round(f * (quick ? quickFiles : fullFiles)), filesTotal: quick ? quickFiles : fullFiles,
        bytesDone: Math.round(f * (quick ? quickBytes : fullBytes)), bytesTotal: quick ? quickBytes : fullBytes,
      });
      if (t < 1) return;
      clearInterval(timer);
      cancelMockDups = null;
      mockDupSearch = { generation, durationMs: Date.now() - started };
      resolve(mockDuplicates(generation));
    }, 100);
    cancelMockDups = () => {
      clearInterval(timer);
      cancelMockDups = null;
      resolve(null);
    };
  });
}

/** Demo Dev Junk, same shape and rules as `junk_projects` in Rust. */
function mockDevJunk(generation: number): DevJunkDto {
  const nowSecs = Math.floor(Date.now() / 1000);
  const projects = new Map<string, JunkProjectDto>();
  for (const n of flatten(mockTree)) {
    const kind = mockKind(n);
    const trail = kind ? findPath(mockTree, n.id) : null;
    const dir = trail?.[trail.length - 2];
    if (!kind || !dir) continue;
    const logical = Math.round(n.sizeBytes * 0.985);
    let p = projects.get(dir.id);
    if (!p) {
      const daysAgo = Math.min(...(dir.children ?? []).filter((c) => !mockKind(c)).map((c) => c.lastModifiedDaysAgo));
      p = { id: mockNodeId(dir), name: dir.name, path: dir.path, newestMtime: Number.isFinite(daysAgo) ? nowSecs - daysAgo * 86_400 : 0, allocatedBytes: 0, logicalBytes: 0, fileCount: 0, artifacts: [] };
      projects.set(dir.id, p);
    }
    p.artifacts.push({ id: mockNodeId(n), name: n.name, relPath: n.name, path: n.path, kind, allocatedBytes: n.sizeBytes, logicalBytes: logical, fileCount: n.fileCount });
    p.allocatedBytes += n.sizeBytes;
    p.logicalBytes += logical;
    p.fileCount += n.fileCount;
  }
  const list = [...projects.values()].sort((a, b) => b.allocatedBytes - a.allocatedBytes);
  list.forEach((p) => p.artifacts.sort((a, b) => b.allocatedBytes - a.allocatedBytes));
  return {
    generation,
    projects: list,
    allocatedBytes: list.reduce((a, p) => a + p.allocatedBytes, 0),
    logicalBytes: list.reduce((a, p) => a + p.logicalBytes, 0),
    artifactCount: list.reduce((a, p) => a + p.artifacts.length, 0),
  };
}

/** Demo search, same rules as `search_nodes` in Rust: every word in the name, largest first. */
function mockSearch(generation: number, query: string, metric: SizeMetricDto, limit: number): SearchResultDto {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  const nowSecs = Math.floor(Date.now() / 1000);
  const parents = new Map<string, FsNode>();
  const walk = (n: FsNode) => n.children?.forEach((c) => { parents.set(c.id, n); walk(c); });
  walk(mockTree);
  const matches = terms.length
    ? flatten(mockTree).filter((n) => n !== mockTree && terms.every((t) => n.name.toLowerCase().includes(t)))
    : [];
  const hits = matches
    .map((n) => ({ node: mockNodeDto(n, metric, nowSecs), parentId: mockNodeId(parents.get(n.id) ?? mockTree) }))
    .sort((a, b) => b.node.value - a.node.value || a.node.name.localeCompare(b.node.name))
    .slice(0, limit);
  return { generation, query, metric, totalMatches: matches.length, hits };
}

// ── Public API ─────────────────────────────────────────────────────────────────
/** Static information about the running application. */
export const appInfo = (): Promise<AppInfoDto> =>
  isTauri() ? call("app_info", {}) : Promise.resolve(mock.appInfo());

/** Mounted volumes that can be scanned. */
export const listDrives = (): Promise<DriveDto[]> =>
  isTauri() ? call("list_drives", {}) : Promise.resolve(mock.drives());

/** Native folder picker; `null` when the user cancels (always `null` in the browser demo). */
export const pickFolder = (): Promise<string | null> =>
  isTauri() ? call("pick_folder", {}) : Promise.resolve(null);

/**
 * Scans `path`, calling `onProgress` up to ~10 times per second. Resolves with the summary,
 * or `null` when the scan was cancelled or replaced by a newer one.
 */
export const startScan = (
  path: string,
  onProgress: (progress: ScanProgressDto) => void,
): Promise<ScanSummaryDto | null> => {
  if (!isTauri()) return mock.scan(path, onProgress);
  const channel = new Channel<ScanProgressDto>();
  channel.onmessage = onProgress;
  return call("start_scan", { path, onProgress: channel });
};

/**
 * Projection of directory `nodeId` (`null` = scan root) from scan `generation`: up to 200
 * largest children plus one "Other" node (`id: null`). Rejects with an `IpcError` for a stale
 * generation or an unknown node. With `delta` (needs a previous scan of the same location),
 * only changed children are listed, biggest growth first, plus the items removed from it.
 */
export const getViewport = (generation: number, nodeId: number | null, metric: SizeMetricDto, delta = false): Promise<ViewportDto> => {
  if (isTauri()) return call("get_viewport", { generation, nodeId, metric, delta });
  try {
    return Promise.resolve(mockViewport(generation, nodeId, metric, delta));
  } catch (e) {
    return Promise.reject(e);
  }
};

/** Dev Junk of scan `generation`: projects with rebuildable artifacts, largest first. */
export const getDevJunk = (generation: number): Promise<DevJunkDto> =>
  isTauri() ? call("get_dev_junk", { generation }) : Promise.resolve(mockDevJunk(generation));

/**
 * Changes view of scan `generation` since the previous scan of the same location, like
 * `git diff --stat`: the largest `limit` changes of `kind` (`null` = all kinds) and totals of
 * every kind. Rejects when there is no previous scan.
 */
export const getChanges = (generation: number, kind: ChangeKindDto | null = null, limit: number | null = null): Promise<ChangesDto> =>
  isTauri() ? call("get_changes", { generation, kind, limit }) : Promise.resolve(mockChanges(generation, kind, limit ?? 500));

/** Safety Advisor verdict ("Can I delete this?") for one node; `system` = the backend refuses to delete it. */
export const getAdvice = (generation: number, nodeId: number): Promise<AdviceDto> =>
  isTauri() ? call("get_advice", { generation, nodeId }) : Promise.resolve(mockAdvice(generation, nodeId));

/**
 * Reviews the Sieve Basket of scan `generation`: Safety Advisor verdicts, items inside another
 * collected folder (counted once), items the backend refuses, and what emptying it frees.
 */
export const reviewSieve = (generation: number, nodeIds: number[]): Promise<SieveDto> =>
  isTauri() ? call("review_sieve", { generation, nodeIds }) : Promise.resolve(mockReviewSieve(generation, nodeIds));

/**
 * `Ctrl+K`: the largest nodes of scan `generation` whose name contains every word of `query`
 * (case-insensitive). Searched in memory — the disk is never touched.
 */
export const searchNodes = (generation: number, query: string, metric: SizeMetricDto, limit = SEARCH_LIMIT): Promise<SearchResultDto> =>
  isTauri() ? call("search_nodes", { generation, query, metric, limit }) : Promise.resolve(mockSearch(generation, query, metric, limit));

/**
 * Searches scan `generation` for files with identical content (reads the files: size, then
 * the first and last 16 KiB, then the full `blake3` hash), calling `onProgress` up to ~10
 * times per second. Resolves with the groups, or `null` when cancelled or superseded.
 */
export const findDuplicates = (
  generation: number,
  onProgress: (progress: DuplicateProgressDto) => void,
): Promise<DuplicatesDto | null> => {
  if (!isTauri()) return mockFindDuplicates(generation, onProgress);
  const channel = new Channel<DuplicateProgressDto>();
  channel.onmessage = onProgress;
  return call("find_duplicates", { generation, onProgress: channel });
};

/** Last duplicate search of scan `generation` without copies trashed since; `null` when none finished. */
export const getDuplicates = (generation: number): Promise<DuplicatesDto | null> =>
  isTauri() ? call("get_duplicates", { generation }) : Promise.resolve(mockDuplicates(generation));

/** Cancels the duplicate search in progress; `false` when nothing was running. */
export const cancelDuplicates = (): Promise<boolean> => {
  if (isTauri()) return call("cancel_duplicates", {});
  const running = cancelMockDups !== null;
  cancelMockDups?.();
  return Promise.resolve(running);
};

/** Cancels the scan in progress; `false` when nothing was running. */
export const cancelScan = (): Promise<boolean> => {
  if (isTauri()) return call("cancel_scan", {});
  const running = cancelMockScan !== null;
  cancelMockScan?.();
  return Promise.resolve(running);
};

// File actions exist only in the desktop app; the browser demo keeps its own in-memory trash
// (see `DiskSieveApp`), so these reject outside Tauri.
const desktopOnly = (command: string) => Promise.reject(new IpcError(command, "available in the desktop app"));

/**
 * Moves nodes of scan `generation` to the system Trash. The backend guard and an on-disk
 * check run for every node; the result has one item per node (a failure never stops the rest).
 */
export const trashNodes = (generation: number, nodeIds: number[]): Promise<TrashReportDto> =>
  isTauri() ? call("trash_nodes", { generation, nodeIds }) : desktopOnly("trash_nodes");

/** Restores nodes moved to Trash in this session ("Undo"; Windows and Linux). */
export const restoreNodes = (generation: number, nodeIds: number[]): Promise<TrashReportDto> =>
  isTauri() ? call("restore_nodes", { generation, nodeIds }) : desktopOnly("restore_nodes");

/** Opens the system file manager with the node selected. */
export const revealInExplorer = (generation: number, nodeId: number): Promise<void> =>
  isTauri() ? call("reveal_in_explorer", { generation, nodeId }).then(() => undefined) : desktopOnly("reveal_in_explorer");

/** Opens the Trash in Finder (macOS). */
export const openTrash = (): Promise<void> =>
  isTauri() ? call("open_trash", {}).then(() => undefined) : desktopOnly("open_trash");
