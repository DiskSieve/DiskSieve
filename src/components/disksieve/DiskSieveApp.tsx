import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { ArrowUp, ChevronRight, Copy, File, Folder, FolderOpen, HardDrive, Layers, Lightbulb, ShieldAlert, ShieldCheck, RefreshCw, ShoppingBasket, Trash2, Eye, ExternalLink } from "lucide-react";
import { flatten, hintFor, initialDrive, removeNode, type FsNode } from "@/data/mockDrive";
import { copyText } from "@/lib/clipboard";
import type { TrashTarget } from "@/lib/trash";
import { formatBytes, formatMtime } from "@/lib/format";
import { legendFor, type ColorMode } from "@/lib/treemap-palette";
import { TitleBar } from "./TitleBar";
import { ScanProgressDialog, ScanTargetDialog, formatDuration, usedBytes } from "./ScanDialogs";
import { SpaceBalanceSheet, SpaceBalanceTile } from "./SpaceBalance";
import { Treemap, otherLabel } from "./Treemap";
import { DevJunkView } from "./DevJunkView";
import { SieveBasket } from "./SieveBasket";
import { LEVELS } from "@/lib/advice";
import { SearchPalette } from "./SearchPalette";
import { DuplicatesView, initialDuplicates, wastedBytes } from "./DuplicatesView";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { PathBreadcrumb } from "./PathBreadcrumb";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "@/components/ui/context-menu";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog";
import { buttonVariants } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Toaster } from "@/components/ui/sonner";
import { cn } from "@/lib/utils";
import { appInfo, cancelScan, getAdvice, getDevJunk, getViewport, isTauri, listDrives, mockNodeId, openTrash, pickFolder, restoreNodes, revealInExplorer, reviewSieve, setMockTree, startScan, trashNodes } from "@/lib/ipc";
import type { AdviceDto, AppInfoDto, DevJunkDto, DriveDto, ScanProgressDto, ScanSummaryDto, SieveDto, SizeMetricDto, TrashReportDto, TreemapNodeDto, ViewportDto } from "@/types/ipc";

type Tab = "explorer" | "junk" | "dups";
const MODES: { key: ColorMode; label: string }[] = [{ key: "extension", label: "By Extension" }, { key: "age", label: "Cold Data (Age)" }, { key: "junk", label: "Dev Junk" }];
const METRICS: { key: SizeMetricDto; label: string; hint: string }[] = [
  { key: "allocated", label: "On disk", hint: "Space actually used on disk (clusters, compression, sparse files)" },
  { key: "logical", label: "Logical", hint: "Apparent file size, as shown by most file managers" },
];
const PICK_FOLDER = "__pick_folder__";

/** Scan in progress: its root, latest progress and the expected total (used space of a drive), if known. */
type ScanState = { path: string; progress: ScanProgressDto | null; expectedBytes: number | null };

const FLAG_LABELS: Record<string, string> = { hardlink: "hard link", cloudOnline: "cloud only", accessDenied: "no access", reparse: "link", otherVolume: "other volume", nameLossy: "non-Unicode name" };

/** Flags the backend guard always refuses to move to Trash. */
const PROTECTED_FLAGS: Record<string, string> = { cloudOnline: "Online-only cloud file — remove it in the cloud app.", reparse: "Links, junctions and mount points are not moved to Trash.", otherVolume: "Belongs to another drive.", nameLossy: "The path contains a non-Unicode name." };

const nodeLabel = (n: TreemapNodeDto) => (n.id === null ? otherLabel(n) : n.name);
const revealLabel = (os: string) => (os === "macos" ? "Reveal in Finder" : os === "linux" ? "Show in File Manager" : "Show in File Explorer");
const protectedReason = (n: TreemapNodeDto) => n.flags.map((f) => PROTECTED_FLAGS[f]).find(Boolean) ?? null;
/** Context-menu hint only — the backend guard decides; the Inspector shows the real verdict. */
const trashBlocked = (n: TreemapNodeDto) => !!protectedReason(n) || hintFor(n.name).startsWith("SYSTEM");
const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

/** Applies a trash (`sign = -1`) or restore (`+1`) report to the scan totals. */
const adjustSummary = (s: ScanSummaryDto | null, r: TrashReportDto, sign: 1 | -1): ScanSummaryDto | null =>
  s && s.generation === r.generation
    ? { ...s, allocatedBytes: s.allocatedBytes + sign * r.allocatedBytes, logicalBytes: s.logicalBytes + sign * r.logicalBytes, fileCount: s.fileCount + sign * r.fileCount }
    : s;

export function DiskSieveApp() {
  // Demo-only data (browser mode): Dev Junk tab, Ctrl+K "largest items" and the in-memory trash.
  const [tree, setTree] = useState<FsNode>(initialDrive);
  const [mode, setMode] = useState<ColorMode>("extension");
  const [metric, setMetric] = useState<SizeMetricDto>("allocated");
  const [delta, setDelta] = useState(false);
  const [tab, setTab] = useState<Tab>("explorer");
  const [inspected, setInspected] = useState<TreemapNodeDto | null>(null);
  const [ctxNode, setCtxNode] = useState<TreemapNodeDto | null>(null);
  const [cmdOpen, setCmdOpen] = useState(false);
  const [dups, setDups] = useState(initialDuplicates);
  const [scan, setScan] = useState<ScanState | null>(null);
  const [summary, setSummary] = useState<ScanSummaryDto | null>(null);
  // "What do you want to scan?" — shown at startup until something is scanned.
  const [targetOpen, setTargetOpen] = useState(true);
  const [drives, setDrives] = useState<DriveDto[]>([]);
  const [root, setRoot] = useState<string | null>(null);
  const scanSeq = useRef(0);
  const [info, setInfo] = useState<AppInfoDto | null>(null);
  const [balanceOpen, setBalanceOpen] = useState(false);
  // Explorer: directory shown (`null` = scan root) and its projection from Rust.
  const [currentId, setCurrentId] = useState<number | null>(null);
  const [viewport, setViewport] = useState<ViewportDto | null>(null);
  const [viewportError, setViewportError] = useState<string | null>(null);
  const viewSeq = useRef(0);
  // Bumped after trash / restore so the projection is fetched again from the updated tree.
  const [treeVersion, setTreeVersion] = useState(0);
  const [confirmTrash, setConfirmTrash] = useState<TrashTarget[] | null>(null);
  // Dev Junk of the current scan (refetched with the viewport after trash / restore).
  const [devJunk, setDevJunk] = useState<DevJunkDto | null>(null);
  const [devJunkError, setDevJunkError] = useState<string | null>(null);
  const generation = summary?.generation ?? null;
  const os = info?.os ?? "";

  useEffect(() => {
    appInfo().then(setInfo).catch((e: Error) => toast.error("Could not reach the DiskSieve engine", { description: e.message }));
    listDrives().then(setDrives).catch((e: Error) => toast.error("Could not list drives", { description: e.message }));
  }, []);

  // Fetch the projection whenever the scan, the directory or the metric changes. Responses
  // that are out of order or belong to another scan generation are dropped.
  useEffect(() => {
    if (generation === null) {
      setViewport(null);
      return;
    }
    if (!isTauri()) setMockTree(tree);
    const seq = ++viewSeq.current;
    getViewport(generation, currentId, metric)
      .then((vp) => {
        if (seq !== viewSeq.current || vp.generation !== generation) return;
        setViewport(vp);
        setViewportError(null);
      })
      .catch((e: Error) => {
        if (seq !== viewSeq.current) return;
        setViewportError(e.message);
      });
  }, [generation, currentId, metric, tree, treeVersion]);
  useEffect(() => {
    if (generation === null) {
      setDevJunk(null);
      return;
    }
    let live = true;
    getDevJunk(generation)
      .then((dj) => {
        if (!live || dj.generation !== generation) return;
        setDevJunk(dj);
        setDevJunkError(null);
      })
      .catch((e: Error) => { if (live) setDevJunkError(e.message); });
    return () => { live = false; };
  }, [generation, tree, treeVersion]);

  // Safety Advisor verdict of the inspected node, refreshed when trash / restore changes the tree.
  const [advice, setAdvice] = useState<AdviceDto | null>(null);
  const [adviceError, setAdviceError] = useState<string | null>(null);
  const inspectedId = inspected?.id ?? null;
  useEffect(() => {
    setAdvice(null);
    setAdviceError(null);
    if (generation === null || inspectedId === null) return;
    let live = true;
    getAdvice(generation, inspectedId)
      .then((a) => { if (live && a.generation === generation) setAdvice(a); })
      .catch((e: unknown) => { if (live) setAdviceError(e instanceof Error ? e.message : String(e)); });
    return () => { live = false; };
  }, [generation, inspectedId, treeVersion]);

  // Sieve Basket: node ids collected for one "Move to Trash", reviewed by the backend.
  const [basket, setBasket] = useState<number[]>([]);
  const [sieve, setSieve] = useState<SieveDto | null>(null);
  const [sieveError, setSieveError] = useState<string | null>(null);
  const [sieveOpen, setSieveOpen] = useState(false);
  useEffect(() => {
    if (generation === null || !basket.length) {
      setSieve(null);
      setSieveError(null);
      return;
    }
    let live = true;
    reviewSieve(generation, basket)
      .then((s) => {
        if (!live || s.generation !== generation) return;
        setSieveError(null);
        // Items moved to Trash meanwhile (also from other views) leave the basket.
        if (s.missing.length) setBasket((b) => b.filter((id) => !s.missing.includes(id)));
        setSieve(s);
      })
      .catch((e: unknown) => { if (live) setSieveError(e instanceof Error ? e.message : String(e)); });
    return () => { live = false; };
  }, [generation, basket, tree, treeVersion]);
  const inSieve = (id: number | null) => id !== null && basket.includes(id);
  const addToSieve = (ids: number[]) => {
    const fresh = ids.filter((id) => !basket.includes(id));
    if (!fresh.length) {
      toast("Already in the Sieve");
      return;
    }
    setBasket((b) => [...b, ...fresh.filter((id) => !b.includes(id))]);
    toast.success(fresh.length === 1 ? "Added to Sieve" : `Added ${fresh.length} items to Sieve`, { action: { label: "Open Sieve", onClick: () => setSieveOpen(true) } });
  };
  const removeFromSieve = (id: number) => setBasket((b) => b.filter((x) => x !== id));
  const toggleSieve = (n: TreemapNodeDto) => {
    if (n.id === null) return;
    if (inSieve(n.id)) removeFromSieve(n.id);
    else addToSieve([n.id]);
  };

  const all = useMemo(() => flatten(tree), [tree]);
  const breadcrumbs = viewport?.breadcrumbs ?? [];
  const valueOf = useCallback((n: TreemapNodeDto) => (delta ? n.deltaBytes ?? 0 : n.value), [delta]);
  const items = useMemo(() => {
    const nodes = viewport?.nodes ?? [];
    return delta ? nodes.filter((n) => (n.deltaBytes ?? 0) > 0).sort((a, b) => valueOf(b) - valueOf(a)) : nodes;
  }, [viewport, delta, valueOf]);
  const folderTotal = items.reduce((a, n) => a + Math.max(valueOf(n), 0), 0) || 1;
  const currentTotal = delta ? items.reduce((a, n) => a + valueOf(n), 0) : viewport?.totalValue ?? 0;
  const itemCount = items.reduce((a, n) => a + (n.id === null ? n.otherCount ?? 0 : 1), 0);

  /** Demo trash: removes nodes from the in-memory demo tree. */
  const trash = useCallback((nodes: FsNode[]) => {
    // Side effects stay outside the state updater (StrictMode runs updaters twice).
    const prev = tree;
    const bytes = nodes.reduce((a, n) => a + n.sizeBytes, 0);
    setTree(nodes.reduce((t, n) => removeNode(t, n.id), prev));
    toast.success(nodes.length === 1 ? `Moved “${nodes[0]!.name}” to Trash` : `Moved ${nodes.length} items to Trash`, {
      description: `${formatBytes(bytes)} · disk space is freed when the Trash is emptied`, action: { label: "Undo", onClick: () => setTree(prev) }, duration: 8000,
    });
    setInspected(null);
  }, [tree]);

  /** "Undo" for a desktop trash: restores the items and puts them back into the tree. */
  const undoTrash = (gen: number, ids: number[]) => {
    restoreNodes(gen, ids)
      .then((report) => {
        setSummary((s) => adjustSummary(s, report, 1));
        setTreeVersion((v) => v + 1);
        const restored = report.items.filter((i) => !i.error);
        const failed = report.items.filter((i) => i.error);
        if (restored.length) toast.success(restored.length === 1 ? `Restored “${baseName(restored[0]!.path)}”` : `Restored ${restored.length} items`);
        if (failed.length) toast.error(failed.length === 1 ? `Could not restore “${baseName(failed[0]!.path)}”` : `${failed.length} items could not be restored`, { description: failed[0]!.error });
      })
      .catch((e: Error) => toast.error("Could not restore from Trash", { description: e.message }));
  };

  /** Moves confirmed nodes to Trash: the backend checks each one and reports per item. */
  const moveToTrash = (nodes: TrashTarget[]) => {
    if (!isTauri()) {
      const demo = nodes.map((n) => all.find((f) => mockNodeId(f) === n.id)).filter((f): f is FsNode => !!f);
      if (demo.length) trash(demo);
      return;
    }
    if (generation === null) return;
    const gen = generation;
    const ids = nodes.flatMap((n) => (n.id === null ? [] : [n.id]));
    trashNodes(gen, ids)
      .then((report) => {
        setSummary((s) => adjustSummary(s, report, -1));
        setTreeVersion((v) => v + 1);
        const moved = report.items.filter((i) => !i.error);
        const failed = report.items.filter((i) => i.error);
        if (moved.length) {
          setInspected((n) => (n && moved.some((m) => m.id === n.id) ? null : n));
          const action = report.undoAvailable ? { label: "Undo", onClick: () => undoTrash(gen, moved.map((m) => m.id)) }
            : os === "macos" ? { label: "Open Trash", onClick: () => void openTrash().catch((e: Error) => toast.error("Could not open the Trash", { description: e.message })) }
            : undefined;
          toast.success(moved.length === 1 ? `Moved “${baseName(moved[0]!.path)}” to Trash` : `Moved ${moved.length} items to Trash`, {
            description: `${formatBytes(report.allocatedBytes)} · disk space is freed when the Trash is emptied`, action, duration: 8000,
          });
        }
        if (failed.length) {
          toast.error(failed.length === 1 ? `“${baseName(failed[0]!.path)}” was not moved to Trash` : `${failed.length} items were not moved to Trash`, { description: failed[0]!.error });
        }
      })
      .catch((e: Error) => toast.error("Could not move to Trash", { description: e.message }));
  };

  const reveal = (n: Pick<TrashTarget, "id" | "path">) => {
    if (n.id === null) return;
    if (!isTauri() || generation === null) {
      toast(`${revealLabel(os)} works in the desktop app`, { description: n.path });
      return;
    }
    revealInExplorer(generation, n.id).catch((e: Error) => toast.error("Could not open the file manager", { description: e.message }));
  };

  const activate = (n: TreemapNodeDto) => {
    if (n.id === null) return;
    if (n.isDir) setCurrentId(n.id);
    else setInspected(n);
  };

  const runScan = useCallback((path: string) => {
    // Only the newest scan may update the UI; an older one resolves with `null` when replaced.
    const seq = ++scanSeq.current;
    const drive = drives.find((d) => d.mountPoint === path);
    setRoot(path);
    setTargetOpen(false);
    setScan({ path, progress: null, expectedBytes: drive ? usedBytes(drive) : null });
    startScan(path, (progress) => {
      if (seq === scanSeq.current) setScan((s) => (s ? { ...s, progress } : s));
    })
      .then((result) => {
        if (seq !== scanSeq.current) return;
        setScan(null);
        if (!result) {
          toast("Scan cancelled", { description: "The previous results are kept." });
          return;
        }
        // Node ids belong to one scan: start the new tree at its root.
        setSummary(result);
        setCurrentId(null);
        setInspected(null);
        setCtxNode(null);
        setBasket([]);
        toast.success("Scan complete", {
          description: `${result.fileCount.toLocaleString()} files · ${formatBytes(result.allocatedBytes)} in ${formatDuration(result.durationMs)}`,
        });
      })
      .catch((e: Error) => {
        if (seq !== scanSeq.current) return;
        setScan(null);
        toast.error("Scan failed", { description: e.message });
      });
  }, [drives]);

  const chooseFolder = async () => {
    if (!isTauri()) {
      toast("Folder picker opens in the desktop build");
      return;
    }
    try {
      const folder = await pickFolder();
      if (folder) runScan(folder);
    } catch (e) {
      toast.error("Could not open the folder picker", { description: (e as Error).message });
    }
  };

  const rescan = () => {
    if (root) runScan(root);
    else setTargetOpen(true);
  };

  const cancel = () => {
    cancelScan().catch((e: Error) => toast.error("Could not cancel the scan", { description: e.message }));
  };

  const onRootSelect = (value: string) => {
    if (value === PICK_FOLDER) void chooseFolder();
    else runScan(value);
  };

  useEffect(() => {
    const h = (e: KeyboardEvent) => { if (e.key.toLowerCase() === "k" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); setCmdOpen((o) => !o); } };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);

  const copyPath = (n: TreemapNodeDto) => {
    copyText(n.path).then(() => toast.success("Path copied", { description: n.path }))
      .catch((e: Error) => toast.error("Could not copy the path", { description: e.message }));
  };

  // Same actions for a treemap tile and a list row (`ctxNode` = item under the cursor).
  const nodeMenu = (
    <ContextMenuContent className="w-60">
      <div className="truncate px-2 py-1.5 font-mono text-[11px] text-muted-foreground">{ctxNode?.name ?? "No item under cursor"}</div>
      <ContextMenuSeparator />
      <ContextMenuItem disabled={!ctxNode} onSelect={() => ctxNode && setInspected(ctxNode)}><Eye className="h-4 w-4" />Inspect & Safety Advisor</ContextMenuItem>
      <ContextMenuItem disabled={!ctxNode} onSelect={() => ctxNode && reveal(ctxNode)}><ExternalLink className="h-4 w-4" />{revealLabel(os)}</ContextMenuItem>
      <ContextMenuItem disabled={!ctxNode} onSelect={() => ctxNode && copyPath(ctxNode)}><Copy className="h-4 w-4" />Copy Full Path</ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem disabled={!ctxNode} onSelect={() => ctxNode && toggleSieve(ctxNode)}><ShoppingBasket className="h-4 w-4" />{ctxNode && inSieve(ctxNode.id) ? "Remove from Sieve" : "Add to Sieve"}</ContextMenuItem>
      <ContextMenuItem disabled={!ctxNode || trashBlocked(ctxNode)} className="text-destructive focus:text-destructive" onSelect={() => ctxNode && setConfirmTrash([ctxNode])}><Trash2 className="h-4 w-4" />Move to Trash…</ContextMenuItem>
    </ContextMenuContent>
  );
  const customRoot = root !== null && !drives.some((d) => d.mountPoint === root) ? root : null;
  const deltaAvailable = !isTauri();

  const emptyMessage = generation === null ? null
    : viewportError ? `Could not load this folder: ${viewportError}`
    : !viewport ? "Loading…"
    : delta ? "No growth since yesterday in this folder."
    : "This folder is empty.";

  return (
    <TooltipProvider delayDuration={200}>
      <div className="flex h-dvh flex-col overflow-hidden bg-background text-foreground">
        <TitleBar info={info} onSearch={() => setCmdOpen(true)} />

        {/* Toolbar: scan target, tabs, Delta */}
        <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border bg-card px-3 py-2 md:gap-3 md:px-4 lg:h-12 lg:flex-nowrap lg:py-0">
          <Select value={root ?? ""} onValueChange={onRootSelect}>
            <SelectTrigger className="h-8 w-auto min-w-0 max-w-[60vw] text-xs sm:w-[270px]"><SelectValue placeholder="Choose a drive or folder…" /></SelectTrigger>
            <SelectContent>
              {drives.map((d) => (
                <SelectItem key={d.mountPoint} value={d.mountPoint}>
                  <span className="flex items-center gap-2">
                    <HardDrive className={cn("h-3.5 w-3.5", d.mountPoint === root && "text-primary")} />
                    <span className="font-mono">{d.mountPoint}</span> {d.name || (d.isRemovable ? "Removable Disk" : "Local Disk")}
                    <span className="font-mono text-muted-foreground">({formatBytes(usedBytes(d))} / {formatBytes(d.totalBytes)})</span>
                    <span className="h-1 w-10 overflow-hidden rounded-full bg-muted"><span className="block h-full bg-primary" style={{ width: `${d.totalBytes ? (usedBytes(d) / d.totalBytes) * 100 : 0}%` }} /></span>
                  </span>
                </SelectItem>
              ))}
              {customRoot && (
                <SelectItem value={customRoot}><span className="flex min-w-0 items-center gap-2"><Folder className="h-3.5 w-3.5 shrink-0 text-primary" /><span className="truncate font-mono">{customRoot}</span></span></SelectItem>
              )}
              <SelectItem value={PICK_FOLDER}><span className="flex items-center gap-2"><FolderOpen className="h-3.5 w-3.5" />Select Folder...</span></SelectItem>
            </SelectContent>
          </Select>
          <Button size="sm" variant="outline" className="h-8" onClick={rescan} disabled={!!scan} aria-label="Rescan">
            <RefreshCw className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">Rescan</span>
          </Button>

          <Tabs value={tab} onValueChange={(v) => setTab(v as Tab)} className="order-last w-full lg:order-none lg:mx-auto lg:w-auto">
            <TabsList className="h-9 w-full lg:w-auto">
              <TabsTrigger value="explorer" className="flex-1 text-xs">Explorer</TabsTrigger>
              <TabsTrigger value="junk" className="flex-1 gap-1.5 text-xs">Dev Junk <Badge className="h-4 px-1.5 font-mono text-[10px]">{devJunk ? formatBytes(devJunk.allocatedBytes) : "—"}</Badge></TabsTrigger>
              <TabsTrigger value="dups" className="flex-1 gap-1.5 text-xs">Duplicates <Badge variant="secondary" className="h-4 px-1.5 font-mono text-[10px]">{formatBytes(wastedBytes(dups))}</Badge></TabsTrigger>
            </TabsList>
          </Tabs>

          <div className="ml-auto flex items-center gap-2 md:gap-3 lg:ml-0">
            <Button size="sm" variant={basket.length ? "default" : "outline"} className="h-8" onClick={() => setSieveOpen(true)} aria-label="Open Sieve">
              <ShoppingBasket className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">Sieve</span>
              {basket.length > 0 && <span className="font-mono text-[11px] tabular-nums">{basket.length} · {sieve ? formatBytes(sieve.allocatedBytes) : "…"}</span>}
            </Button>
            <Tooltip><TooltipTrigger asChild>
              <label className={cn("flex items-center gap-2 text-xs text-muted-foreground", deltaAvailable ? "cursor-pointer" : "cursor-not-allowed opacity-50")}>
                <Switch checked={delta} onCheckedChange={setDelta} disabled={!deltaAvailable} />
                <span className={cn(delta && "text-primary")}><span className="hidden xl:inline">Delta Mode (vs. Yesterday)</span><span className="xl:hidden">Delta</span></span>
              </label>
            </TooltipTrigger>{!deltaAvailable && <TooltipContent className="text-xs">Delta Mode arrives with scan snapshots in v0.2.0</TooltipContent>}</Tooltip>
          </div>
        </div>

        {tab === "explorer" && (
          <>
            {/* Sub-toolbar */}
            <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-2 md:gap-3 md:px-4 lg:h-14 lg:flex-nowrap lg:py-0">
              <Button size="icon" variant="ghost" className="h-8 w-8" disabled={breadcrumbs.length < 2} onClick={() => setCurrentId(breadcrumbs[breadcrumbs.length - 2]!.id)} aria-label="Up one level"><ArrowUp className="h-4 w-4" /></Button>
              <PathBreadcrumb crumbs={breadcrumbs} onNavigate={setCurrentId} />
              <div className="flex max-w-full overflow-x-auto rounded-md border border-border bg-card p-0.5" role="group" aria-label="Size metric">
                {METRICS.map((m) => (
                  <Tooltip key={m.key}><TooltipTrigger asChild>
                    <button onClick={() => setMetric(m.key)} disabled={delta} aria-pressed={metric === m.key}
                      className={cn("whitespace-nowrap rounded px-2 py-1 text-xs transition-colors disabled:opacity-50 sm:px-3", metric === m.key ? "bg-primary/15 text-primary" : "text-muted-foreground hover:text-foreground")}>{m.label}</button>
                  </TooltipTrigger><TooltipContent className="text-xs">{m.hint}</TooltipContent></Tooltip>
                ))}
              </div>
              <div className="flex max-w-full overflow-x-auto rounded-md border border-border bg-card p-0.5" role="group" aria-label="Color mode">
                {MODES.map((m) => (
                  <button key={m.key} onClick={() => setMode(m.key)} aria-pressed={mode === m.key} className={cn("whitespace-nowrap rounded px-2 py-1 text-xs transition-colors sm:px-3", mode === m.key ? "bg-primary/15 text-primary" : "text-muted-foreground hover:text-foreground")}>{m.label}</button>
                ))}
              </div>
              <div className="hidden flex-1 items-center justify-end gap-2 md:flex">
                <Tooltip><TooltipTrigger asChild>
                  <span className="max-w-[340px] truncate rounded-full border border-border bg-card px-3 py-1 font-mono text-[11px] tabular-nums text-muted-foreground">
                    {scan ? `${(scan.progress?.files ?? 0).toLocaleString()} files • ${formatBytes(scan.progress?.allocatedBytes ?? 0)} • scanning…`
                      : summary ? `${summary.fileCount.toLocaleString()} files • Scanned in ${formatDuration(summary.durationMs)}`
                      : "Not scanned yet"}
                  </span>
                </TooltipTrigger><TooltipContent className="max-w-[420px] break-all font-mono text-xs">
                  {scan ? scan.progress?.currentDir || "Starting…" : summary ? `${summary.rootPath} · ${summary.dirCount.toLocaleString()} folders · ${formatBytes(summary.allocatedBytes)} on disk` : "Pick a drive or folder above"}
                </TooltipContent></Tooltip>
                {scan ? (scan.progress?.skippedEntries ?? 0) > 0 && (
                  <Badge variant="outline" className="border-warning/40 font-mono text-[10px] tabular-nums text-warning">{scan.progress!.skippedEntries.toLocaleString()} skipped (Access Denied)</Badge>
                ) : summary && <SpaceBalanceTile summary={summary} onOpen={() => setBalanceOpen(true)} />}
              </div>
            </div>

            {/* Workspace */}
            {generation === null ? (
              <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
                <HardDrive className="h-10 w-10 text-primary" />
                <p className="text-sm font-medium">Nothing scanned yet</p>
                <p className="max-w-sm text-xs text-muted-foreground">Scan a whole drive or a single folder. The scan only reads file metadata — nothing is opened or changed.</p>
                <Button size="sm" onClick={() => setTargetOpen(true)} disabled={!!scan}><FolderOpen className="h-4 w-4" />Choose what to scan…</Button>
              </div>
            ) : (
            <div className="flex min-h-0 flex-1 flex-col overflow-y-auto lg:flex-row lg:overflow-hidden">
              <div className="flex h-[55vh] shrink-0 flex-col border-b border-border p-3 lg:order-2 lg:h-auto lg:w-[68%] lg:border-b-0 lg:border-l">
                <ContextMenu>
                  <ContextMenuTrigger asChild>
                    <div className="min-h-0 flex-1 overflow-hidden rounded-md">
                      {items.length ? <Treemap parentName={breadcrumbs[breadcrumbs.length - 1]?.name ?? ""} items={items} valueOf={valueOf} mode={mode} signed={delta} onActivate={activate} onContextTarget={setCtxNode} />
                        : <div className="flex h-full items-center justify-center px-4 text-center text-sm text-muted-foreground">{emptyMessage}</div>}
                    </div>
                  </ContextMenuTrigger>
                  {nodeMenu}
                </ContextMenu>
                <div className="mt-2 flex flex-wrap items-center gap-4 text-[11px] text-muted-foreground">
                  {legendFor(mode).map((l) => (
                    <span key={l.label} className="flex items-center gap-1.5"><span className="h-2.5 w-2.5 rounded-sm" style={{ background: l.color }} />{l.label}</span>
                  ))}
                  <span className="ml-auto hidden font-mono sm:inline">{delta ? "showing growth since yesterday" : "right-click a tile for actions"}</span>
                </div>
              </div>

              <div className="flex min-h-[50vh] flex-col lg:order-1 lg:min-h-0 lg:w-[32%]">
                <div className="flex items-center justify-between border-b border-border px-4 py-2.5 text-xs text-muted-foreground">
                  <span className="tabular-nums">{itemCount.toLocaleString()} items · sorted by {delta ? "growth" : metric === "allocated" ? "size on disk" : "logical size"}</span>
                  <span className="font-mono tabular-nums text-foreground">{formatBytes(currentTotal, delta)}</span>
                </div>
                <ContextMenu>
                <ContextMenuTrigger asChild>
                <div className="min-h-0 flex-1 overflow-auto" onContextMenu={(e) => { if (e.target === e.currentTarget) setCtxNode(null); }}>
                  {items.map((n) => {
                    const pct = (Math.max(valueOf(n), 0) / folderTotal) * 100;
                    const isOther = n.id === null;
                    return (
                      <button key={n.id ?? "other"} onClick={() => activate(n)} disabled={isOther}
                        onContextMenu={() => setCtxNode(isOther ? null : n)}
                        className="group relative flex w-full items-center gap-2.5 border-b border-border/60 px-4 py-2 text-left text-sm hover:bg-accent/40 disabled:cursor-default disabled:hover:bg-transparent">
                        <span className="absolute inset-y-0 left-0 bg-primary/10" style={{ width: `${pct}%` }} />
                        {isOther ? <Layers className="relative h-4 w-4 shrink-0 text-muted-foreground" />
                          : n.isDir ? <Folder className="relative h-4 w-4 shrink-0 text-primary" /> : <File className="relative h-4 w-4 shrink-0 text-muted-foreground" />}
                        <span className={cn("relative min-w-0 flex-1 truncate", isOther && "text-muted-foreground")}>{nodeLabel(n)}</span>
                        {n.junkKind && <Badge variant="outline" className="relative h-4 border-primary/40 px-1.5 font-mono text-[9px] text-primary">{n.junkKind}</Badge>}
                        {n.flags.filter((f) => FLAG_LABELS[f]).slice(0, 1).map((f) => (
                          <Badge key={f} variant="outline" className="relative h-4 border-warning/40 px-1.5 text-[9px] text-warning">{FLAG_LABELS[f]}</Badge>
                        ))}
                        <span className="relative hidden w-14 shrink-0 text-right font-mono text-[11px] tabular-nums sm:inline text-muted-foreground">{formatMtime(n.newestMtime)}</span>
                        <span className={cn("relative w-20 shrink-0 text-right font-mono text-xs tabular-nums", delta && "text-primary")}>{formatBytes(valueOf(n), delta)}</span>
                        {n.isDir ? <ChevronRight className="relative h-3.5 w-3.5 text-muted-foreground opacity-0 group-hover:opacity-100" /> : <span className="w-3.5" />}
                      </button>
                    );
                  })}
                </div>
                </ContextMenuTrigger>
                {nodeMenu}
                </ContextMenu>
              </div>
            </div>
            )}
          </>
        )}
        {tab === "junk" && <DevJunkView data={devJunk} error={devJunkError} scanned={generation !== null} revealLabel={revealLabel(os)}
          onTrash={setConfirmTrash} onSieve={addToSieve} onReveal={(id, path) => reveal({ id, path })} onScan={() => setTargetOpen(true)} />}
        {tab === "dups" && <DuplicatesView groups={dups} setGroups={setDups} />}

        {/* Inspector */}
        <Sheet open={!!inspected} onOpenChange={(o) => !o && setInspected(null)}>
          <SheetContent className="w-full overflow-y-auto sm:w-[420px] sm:max-w-[420px]">
            {inspected && <Inspector n={inspected} os={os} advice={advice?.nodeId === inspected.id ? advice : null} adviceError={adviceError} inSieve={inSieve(inspected.id)} onSieve={() => toggleSieve(inspected)} onTrash={() => setConfirmTrash([inspected])} onCopy={() => copyPath(inspected)} onReveal={() => reveal(inspected)} />}
          </SheetContent>
        </Sheet>

        <SieveBasket open={sieveOpen} onOpenChange={setSieveOpen} count={basket.length} sieve={sieve} error={sieveError}
          onRemove={removeFromSieve} onClear={() => setBasket([])} onInspect={setInspected} onTrash={setConfirmTrash} />

        <TrashConfirmDialog nodes={confirmTrash} os={os} onCancel={() => setConfirmTrash(null)}
          onConfirm={() => { if (confirmTrash) moveToTrash(confirmTrash); setConfirmTrash(null); }} />

        <SpaceBalanceSheet summary={summary} open={balanceOpen} onOpenChange={setBalanceOpen} os={os} />

        <SearchPalette open={cmdOpen} onOpenChange={setCmdOpen} generation={generation} metric={metric} treeVersion={treeVersion}
          onOpenHit={(h) => { setTab("explorer"); setCurrentId(h.parentId); setInspected(h.node); setCmdOpen(false); }}
          actions={[
            ...MODES.map((m) => ({ label: `Color by: ${m.label}`, run: () => { setMode(m.key); setTab("explorer"); setCmdOpen(false); } })),
            ...METRICS.map((m) => ({ label: `Size: ${m.label}`, run: () => { setMetric(m.key); setTab("explorer"); setCmdOpen(false); } })),
            ...(deltaAvailable ? [{ label: "Toggle Delta Mode", run: () => { setDelta((d) => !d); setCmdOpen(false); } }] : []),
            { label: "Open Dev Junk cleaner", run: () => { setTab("junk"); setCmdOpen(false); } },
            { label: "Open Sieve", run: () => { setSieveOpen(true); setCmdOpen(false); } },
            { label: "Open Duplicates finder", run: () => { setTab("dups"); setCmdOpen(false); } },
            { label: "Rescan drive", run: () => { setCmdOpen(false); rescan(); } },
          ]} />
        <ScanTargetDialog open={targetOpen && !scan} onOpenChange={setTargetOpen} drives={drives} onPickDrive={runScan} onPickFolder={() => void chooseFolder()} />
        <ScanProgressDialog path={scan?.path ?? null} progress={scan?.progress ?? null} expectedBytes={scan?.expectedBytes ?? null} onCancel={cancel} />
        <Toaster position="bottom-right" theme="dark" />
      </div>
    </TooltipProvider>
  );
}

function TrashConfirmDialog({ nodes, os, onCancel, onConfirm }: { nodes: TrashTarget[] | null; os: string; onCancel: () => void; onConfirm: () => void }) {
  const list = nodes ?? [];
  const bytes = list.reduce((a, n) => a + n.allocatedSize, 0);
  const files = list.reduce((a, n) => a + n.fileCount, 0);
  const title = list.length === 1 ? `Move “${list[0]!.name}” to Trash?` : `Move ${list.length} items to Trash?`;
  const undo = os === "macos" ? "You can put it back from the Trash in Finder." : "You can undo this right after, or restore it from the Trash later.";
  return (
    <AlertDialog open={!!nodes} onOpenChange={(o) => !o && onCancel()}>
      <AlertDialogContent className="grid-cols-[minmax(0,1fr)]">
        <AlertDialogHeader>
          <AlertDialogTitle className="break-all">{title}</AlertDialogTitle>
          <AlertDialogDescription>
            DiskSieve checks that nothing changed since the scan, then moves it to the system Trash. Nothing is deleted permanently. {undo}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="space-y-1 rounded-md border border-border bg-background px-3 py-2">
          {list.slice(0, 5).map((n) => (
            <div key={n.id ?? n.path} className="truncate font-mono text-[11px] text-muted-foreground" title={n.path}>{n.path}</div>
          ))}
          {list.length > 5 && <div className="text-[11px] text-muted-foreground">and {list.length - 5} more</div>}
          <div className="flex justify-between border-t border-border pt-1.5 font-mono text-xs tabular-nums">
            <span className="text-muted-foreground">{files.toLocaleString()} {files === 1 ? "file" : "files"}</span>
            <span>{formatBytes(bytes)}</span>
          </div>
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction className={buttonVariants({ variant: "destructive" })} onClick={onConfirm}><Trash2 className="h-4 w-4" />Move to Trash</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** Text with `backtick` spans rendered as inline code. */
function RichText({ text }: { text: string }) {
  return <>{text.split("`").map((part, i) => (i % 2 ? <code key={i} className="rounded bg-muted px-1 font-mono text-[0.85em] text-foreground">{part}</code> : <Fragment key={i}>{part}</Fragment>))}</>;
}

function copyCommand(command: string) {
  copyText(command).then(() => toast.success("Command copied", { description: command }))
    .catch((e: Error) => toast.error("Could not copy the command", { description: e.message }));
}

function Inspector({ n, os, advice, adviceError, inSieve, onSieve, onTrash, onCopy, onReveal }: { n: TreemapNodeDto; os: string; advice: AdviceDto | null; adviceError: string | null; inSieve: boolean; onSieve: () => void; onTrash: () => void; onCopy: () => void; onReveal: () => void }) {
  const blocked = advice?.protectedReason ?? protectedReason(n);
  const ui = advice ? LEVELS[advice.level] : null;
  const rows: [string, string][] = [
    ["Path", n.path],
    ["Size on Disk", `${n.allocatedSize.toLocaleString()} B`],
    ["Logical Size", `${n.logicalSize.toLocaleString()} B`],
    ["IEC Size", formatBytes(n.allocatedSize).replace(" GB", " GiB").replace(" MB", " MiB").replace(" KB", " KiB").replace(" TB", " TiB")],
    ...(n.deltaBytes !== null ? [["Growth (24h)", formatBytes(n.deltaBytes, true)] as [string, string]] : []),
    ["File Count", n.fileCount.toLocaleString()],
    ["Last Modified", formatMtime(n.newestMtime)],
    ["Extension", n.isDir ? "— (folder)" : n.extension ? `.${n.extension}` : "—"],
    ...(n.flags.length ? [["Flags", n.flags.map((f) => FLAG_LABELS[f] ?? f).join(", ")] as [string, string]] : []),
  ];
  return (
    <div className="flex h-full flex-col gap-5">
      <SheetHeader>
        <SheetTitle className="flex items-center gap-2 break-all">{n.isDir ? <Folder className="h-5 w-5 shrink-0 text-primary" /> : <File className="h-5 w-5 shrink-0" />}{n.name}</SheetTitle>
        <SheetDescription className="font-mono text-2xl tabular-nums text-foreground">{formatBytes(n.allocatedSize)}</SheetDescription>
      </SheetHeader>
      <dl className="grid grid-cols-[110px_1fr] gap-x-3 gap-y-2 rounded-lg border border-border bg-background p-4 text-xs">
        {rows.map(([k, v]) => (<Fragment key={k}><dt className="text-muted-foreground">{k}</dt><dd className="break-all font-mono tabular-nums">{v}</dd></Fragment>))}
      </dl>
      <div className={cn("rounded-lg border p-4", ui?.box ?? "border-border bg-background")}>
        <div className="mb-2 flex items-center gap-2 text-sm font-semibold">
          {advice?.level === "safe" ? <ShieldCheck className="h-4 w-4 text-primary" /> : <ShieldAlert className={cn("h-4 w-4", ui?.icon ?? "text-muted-foreground")} />}
          Can I delete this?
          {ui && <Badge className={cn("ml-auto", ui.pill)}>{ui.badge}</Badge>}
        </div>
        {advice ? (
          <>
            <p className="text-sm font-medium">{advice.title}</p>
            <p className="mt-1 text-sm text-muted-foreground"><RichText text={advice.description} /></p>
            {advice.alternative && (
              <div className="mt-3 rounded-md border border-border bg-background p-3">
                <div className="mb-1 flex items-center gap-1.5 text-xs font-semibold"><Lightbulb className="h-3.5 w-3.5 text-primary" />Safer way</div>
                <p className="text-xs text-muted-foreground">{advice.alternative.how}</p>
                {advice.alternative.command && (
                  <div className="mt-2 flex items-center gap-1.5">
                    <code className="min-w-0 flex-1 truncate rounded bg-muted px-2 py-1 font-mono text-[11px]" title={advice.alternative.command}>{advice.alternative.command}</code>
                    <Button size="icon" variant="ghost" className="h-7 w-7 shrink-0" aria-label="Copy command" onClick={() => copyCommand(advice.alternative!.command!)}><Copy className="h-3.5 w-3.5" /></Button>
                  </div>
                )}
              </div>
            )}
          </>
        ) : (
          <p className="text-sm text-muted-foreground">{adviceError ? `Could not check this item: ${adviceError}` : "Checking…"}</p>
        )}
      </div>
      <div className="mt-auto space-y-2">
        {blocked && <p className="flex items-center gap-2 text-xs text-muted-foreground"><ShieldAlert className="h-3.5 w-3.5 shrink-0 text-warning" />{blocked}</p>}
        <div className="flex gap-2">
          <Button variant="outline" className="flex-1" onClick={onReveal}><ExternalLink className="h-4 w-4" />{revealLabel(os)}</Button>
          <Button variant="outline" className="flex-1" onClick={onCopy}><Copy className="h-4 w-4" />Copy Path</Button>
        </div>
        <Button variant="outline" className="w-full" onClick={onSieve}><ShoppingBasket className="h-4 w-4" />{inSieve ? "Remove from Sieve" : "Add to Sieve"}</Button>
        <Button variant="destructive" className="w-full" disabled={advice?.level === "system" || !!blocked} onClick={onTrash}><Trash2 className="h-4 w-4" />Move to Trash…</Button>
      </div>
    </div>
  );
}
