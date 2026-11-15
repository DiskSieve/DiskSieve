import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { ArrowUp, ChevronRight, Copy, File, Folder, FolderOpen, HardDrive, Layers, ShieldAlert, ShieldCheck, RefreshCw, Trash2, Eye, ExternalLink } from "lucide-react";
import { findPath, flatten, hintFor, initialDrive, removeNode, type FsNode, type JunkTag } from "@/data/mockDrive";
import { formatBytes, formatMtime } from "@/lib/format";
import { legendFor, type ColorMode } from "@/lib/treemap-palette";
import { TitleBar } from "./TitleBar";
import { ScanProgressDialog, ScanTargetDialog, formatDuration, usedBytes } from "./ScanDialogs";
import { Treemap, otherLabel } from "./Treemap";
import { DevJunkView, type JunkRow } from "./DevJunkView";
import { DuplicatesView, initialDuplicates, wastedBytes } from "./DuplicatesView";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Breadcrumb, BreadcrumbItem, BreadcrumbLink, BreadcrumbList, BreadcrumbPage, BreadcrumbSeparator } from "@/components/ui/breadcrumb";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "@/components/ui/context-menu";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandShortcut } from "@/components/ui/command";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Toaster } from "@/components/ui/sonner";
import { cn } from "@/lib/utils";
import { appInfo, cancelScan, demoNodeDto, getViewport, isTauri, listDrives, mockNodeId, pickFolder, setMockTree, startScan } from "@/lib/ipc";
import type { AppInfoDto, DriveDto, ScanProgressDto, ScanSummaryDto, SizeMetricDto, TreemapNodeDto, ViewportDto } from "@/types/ipc";

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

const nodeLabel = (n: TreemapNodeDto) => (n.id === null ? otherLabel(n) : n.name);

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
  // Explorer: directory shown (`null` = scan root) and its projection from Rust.
  const [currentId, setCurrentId] = useState<number | null>(null);
  const [viewport, setViewport] = useState<ViewportDto | null>(null);
  const [viewportError, setViewportError] = useState<string | null>(null);
  const viewSeq = useRef(0);
  const generation = summary?.generation ?? null;

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
  }, [generation, currentId, metric, tree]);

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
  const junkRows: JunkRow[] = useMemo(() => all.filter((n) => n.junkTag && n.path.includes("\\Projects\\")).map((n) => ({ node: n, project: n.path.split("\\").slice(-2)[0] ?? "" })), [all]);
  const junkTotal = junkRows.reduce((a, r) => a + r.node.sizeBytes, 0);

  /** Demo trash: removes nodes from the in-memory demo tree. */
  const trash = useCallback((nodes: FsNode[]) => {
    setTree((prev) => {
      const next = nodes.reduce((t, n) => removeNode(t, n.id), prev);
      const bytes = nodes.reduce((a, n) => a + n.sizeBytes, 0);
      setTimeout(() => toast.success(nodes.length === 1 ? `Moved “${nodes[0]!.name}” to Trash` : `Moved ${nodes.length} items to Trash`, {
        description: `${formatBytes(bytes)} reclaimed`, action: { label: "Undo", onClick: () => setTree(prev) },
      }));
      return next;
    });
    setInspected(null);
  }, []);

  const trashNodes = (nodes: TreemapNodeDto[]) => {
    if (isTauri()) {
      toast("Move to Trash is not wired up yet", { description: "Safe deletion with backend checks arrives in the next update." });
      return;
    }
    const demo = nodes.map((n) => all.find((f) => mockNodeId(f) === n.id)).filter((f): f is FsNode => !!f);
    if (demo.length) trash(demo);
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

  const copyPath = (n: TreemapNodeDto) => { navigator.clipboard?.writeText(n.path).catch(() => {}); toast("Path copied", { description: n.path }); };
  const largestFiles = useMemo(() => (isTauri() ? [] : all.filter((n) => !n.isDir || n.junkTag).sort((a, b) => b.sizeBytes - a.sizeBytes).slice(0, 12)), [all]);
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
              <TabsTrigger value="junk" className="flex-1 gap-1.5 text-xs">Dev Junk <Badge className="h-4 px-1.5 font-mono text-[10px]">{formatBytes(junkTotal)}</Badge></TabsTrigger>
              <TabsTrigger value="dups" className="flex-1 gap-1.5 text-xs">Duplicates <Badge variant="secondary" className="h-4 px-1.5 font-mono text-[10px]">{formatBytes(wastedBytes(dups))}</Badge></TabsTrigger>
            </TabsList>
          </Tabs>

          <div className="ml-auto flex items-center gap-2 md:gap-3 lg:ml-0">
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
              <Breadcrumb className="min-w-0 flex-1 overflow-x-auto">
                <BreadcrumbList className="flex-nowrap whitespace-nowrap text-xs">
                  {breadcrumbs.map((b, i) => (
                    <span key={b.id} className="contents">
                      {i > 0 && <BreadcrumbSeparator />}
                      <BreadcrumbItem>
                        {i === breadcrumbs.length - 1 ? <BreadcrumbPage className="font-mono">{b.name}</BreadcrumbPage>
                          : <BreadcrumbLink className="cursor-pointer font-mono" onClick={() => setCurrentId(b.id)}>{b.name}</BreadcrumbLink>}
                      </BreadcrumbItem>
                    </span>
                  ))}
                </BreadcrumbList>
              </Breadcrumb>
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
                {(() => {
                  const skipped = scan ? scan.progress?.skippedEntries ?? 0 : summary?.skippedEntries ?? 0;
                  return skipped > 0 && (
                    <Tooltip><TooltipTrigger asChild>
                      <Badge variant="outline" className="border-warning/40 font-mono text-[10px] tabular-nums text-warning">{skipped.toLocaleString()} skipped (Access Denied)</Badge>
                    </TooltipTrigger><TooltipContent className="max-w-[320px] text-xs">Entries without read permission are counted but not scanned.</TooltipContent></Tooltip>
                  );
                })()}
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
                  <ContextMenuContent className="w-60">
                    <div className="truncate px-2 py-1.5 font-mono text-[11px] text-muted-foreground">{ctxNode?.name ?? "No item under cursor"}</div>
                    <ContextMenuSeparator />
                    <ContextMenuItem disabled={!ctxNode} onSelect={() => ctxNode && setInspected(ctxNode)}><Eye className="h-4 w-4" />Inspect & Safety Advisor</ContextMenuItem>
                    <ContextMenuItem disabled={!ctxNode} onSelect={() => toast("Opening in File Explorer", { description: ctxNode?.path })}><ExternalLink className="h-4 w-4" />Open in File Explorer</ContextMenuItem>
                    <ContextMenuItem disabled={!ctxNode} onSelect={() => ctxNode && copyPath(ctxNode)}><Copy className="h-4 w-4" />Copy Full Path</ContextMenuItem>
                    <ContextMenuSeparator />
                    <ContextMenuItem disabled={!ctxNode} className="text-destructive focus:text-destructive" onSelect={() => ctxNode && trashNodes([ctxNode])}><Trash2 className="h-4 w-4" />Move to Trash</ContextMenuItem>
                  </ContextMenuContent>
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
                <div className="scrollbar-hover min-h-0 flex-1 overflow-auto">
                  {items.map((n) => {
                    const pct = (Math.max(valueOf(n), 0) / folderTotal) * 100;
                    const isOther = n.id === null;
                    return (
                      <button key={n.id ?? "other"} onClick={() => activate(n)} disabled={isOther}
                        onContextMenu={(e) => { e.preventDefault(); if (!isOther) setInspected(n); }}
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
              </div>
            </div>
            )}
          </>
        )}
        {tab === "junk" && <DevJunkView rows={junkRows} onTrash={trash} />}
        {tab === "dups" && <DuplicatesView groups={dups} setGroups={setDups} />}

        {/* Inspector */}
        <Sheet open={!!inspected} onOpenChange={(o) => !o && setInspected(null)}>
          <SheetContent className="w-full overflow-y-auto sm:w-[420px] sm:max-w-[420px]">
            {inspected && <Inspector n={inspected} onTrash={() => trashNodes([inspected])} onCopy={() => copyPath(inspected)} />}
          </SheetContent>
        </Sheet>

        {/* Command palette */}
        <CommandDialog open={cmdOpen} onOpenChange={setCmdOpen}>
          <CommandInput placeholder="Search files, folders, or actions…" />
          <CommandList>
            <CommandEmpty>No results.</CommandEmpty>
            {largestFiles.length > 0 && (
              <CommandGroup heading="Largest items on drive">
                {largestFiles.map((n) => (
                  <CommandItem key={n.id} value={`${n.name} ${n.path}`} onSelect={() => {
                    const p = findPath(tree, n.id);
                    if (p && p.length > 1) setCurrentId(mockNodeId(p[p.length - 2]!));
                    setTab("explorer"); setInspected(demoNodeDto(n)); setCmdOpen(false);
                  }}>
                    {n.isDir ? <Folder className="text-primary" /> : <File />}
                    <span className="truncate">{n.name}</span>
                    <span className="ml-2 truncate font-mono text-[10px] text-muted-foreground">{n.path}</span>
                    <CommandShortcut className="font-mono">{formatBytes(n.sizeBytes)}</CommandShortcut>
                  </CommandItem>
                ))}
              </CommandGroup>
            )}
            <CommandGroup heading="View">
              {MODES.map((m) => <CommandItem key={m.key} onSelect={() => { setMode(m.key); setTab("explorer"); setCmdOpen(false); }}>Color by: {m.label}</CommandItem>)}
              {METRICS.map((m) => <CommandItem key={m.key} onSelect={() => { setMetric(m.key); setTab("explorer"); setCmdOpen(false); }}>Size: {m.label}</CommandItem>)}
              {deltaAvailable && <CommandItem onSelect={() => { setDelta((d) => !d); setCmdOpen(false); }}>Toggle Delta Mode</CommandItem>}
              <CommandItem onSelect={() => { setTab("junk"); setCmdOpen(false); }}>Open Dev Junk cleaner</CommandItem>
              <CommandItem onSelect={() => { setTab("dups"); setCmdOpen(false); }}>Open Duplicates finder</CommandItem>
              <CommandItem onSelect={() => { setCmdOpen(false); rescan(); }}>Rescan drive</CommandItem>
            </CommandGroup>
          </CommandList>
        </CommandDialog>
        <ScanTargetDialog open={targetOpen && !scan} onOpenChange={setTargetOpen} drives={drives} onPickDrive={runScan} onPickFolder={() => void chooseFolder()} />
        <ScanProgressDialog path={scan?.path ?? null} progress={scan?.progress ?? null} expectedBytes={scan?.expectedBytes ?? null} onCancel={cancel} />
        <Toaster position="bottom-right" theme="dark" />
      </div>
    </TooltipProvider>
  );
}

function Inspector({ n, onTrash, onCopy }: { n: TreemapNodeDto; onTrash: () => void; onCopy: () => void }) {
  const hint = hintFor(n.name, (n.junkKind ?? null) as JunkTag);
  const system = hint.startsWith("SYSTEM");
  const safe = hint.startsWith("Safe");
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
      <div className={cn("rounded-lg border p-4", safe ? "border-primary/40 bg-primary/10" : system ? "border-destructive/40 bg-destructive/10" : "border-warning/40 bg-warning/10")}>
        <div className="mb-2 flex items-center gap-2 text-sm font-semibold">
          {safe ? <ShieldCheck className="h-4 w-4 text-primary" /> : <ShieldAlert className={cn("h-4 w-4", system ? "text-destructive" : "text-warning")} />}
          Can I delete this?
          <Badge className={cn("ml-auto", safe ? "" : system ? "bg-destructive text-destructive-foreground" : "bg-warning text-warning-foreground")}>{safe ? "Safe to delete" : system ? "Do not delete" : "Review first"}</Badge>
        </div>
        <p className="text-sm text-muted-foreground">{hint.replace(/^SYSTEM: /, "")}</p>
      </div>
      <div className="mt-auto flex gap-2">
        <Button variant="outline" className="flex-1" onClick={onCopy}><Copy className="h-4 w-4" />Copy Path</Button>
        <Button variant="destructive" className="flex-1" disabled={system} onClick={onTrash}><Trash2 className="h-4 w-4" />Move to Trash</Button>
      </div>
    </div>
  );
}
