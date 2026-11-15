import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
import { ChevronRight, ExternalLink, FolderOpen, Info, Sparkles, Trash2 } from "lucide-react";
import type { DevJunkDto, JunkArtifactDto, JunkProjectDto } from "@/types/ipc";
import { daysSince, formatBytes, formatMtime } from "@/lib/format";
import { junkLabel } from "@/lib/junk";
import type { TrashTarget } from "@/lib/trash";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

/** A project counts as "cold" when nothing outside its artifacts changed for this long. */
const COLD_DAYS = 90;
const ALL = "all";
const COLD = "cold";

const toTarget = (a: JunkArtifactDto): TrashTarget => ({ id: a.id, name: a.name, path: a.path, allocatedSize: a.allocatedBytes, fileCount: a.fileCount });
const sum = (artifacts: JunkArtifactDto[]) => artifacts.reduce((n, a) => n + a.allocatedBytes, 0);
const isCold = (p: JunkProjectDto) => p.newestMtime > 0 && daysSince(p.newestMtime) > COLD_DAYS;

interface Props {
  data: DevJunkDto | null;
  error: string | null;
  scanned: boolean;
  revealLabel: string;
  onTrash: (targets: TrashTarget[]) => void;
  onReveal: (id: number, path: string) => void;
  onScan: () => void;
}

export function DevJunkView({ data, error, scanned, revealLabel, onTrash, onReveal, onScan }: Props) {
  const [filter, setFilter] = useState(ALL);
  const [selected, setSelected] = useState<Set<number>>(() => new Set());
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set());
  const projects = useMemo(() => data?.projects ?? [], [data]);

  // A new scan has new node ids: start over. A refresh after Trash keeps what still exists.
  const generation = data?.generation ?? null;
  useEffect(() => {
    setSelected(new Set());
    setExpanded(new Set());
    setFilter(ALL);
  }, [generation]);
  useEffect(() => {
    const ids = new Set(projects.map((p) => p.id));
    setSelected((s) => (Array.from(s).every((id) => ids.has(id)) ? s : new Set(Array.from(s).filter((id) => ids.has(id)))));
  }, [projects]);

  // Kinds present in this scan, largest total first (filter chips).
  const kinds = useMemo(() => {
    const bytes = new Map<string, number>();
    projects.forEach((p) => p.artifacts.forEach((a) => bytes.set(a.kind, (bytes.get(a.kind) ?? 0) + a.allocatedBytes)));
    return Array.from(bytes.entries()).sort((a, b) => b[1] - a[1]);
  }, [projects]);

  // Projects with the artifacts that match the filter (a kind filter narrows the artifacts).
  const visible = useMemo(() => projects
    .filter((p) => filter !== COLD || isCold(p))
    .map((p) => ({ project: p, artifacts: filter === ALL || filter === COLD ? p.artifacts : p.artifacts.filter((a) => a.kind === filter) }))
    .filter((r) => r.artifacts.length > 0)
    .map((r) => ({ ...r, bytes: sum(r.artifacts) }))
    .sort((a, b) => b.bytes - a.bytes), [projects, filter]);

  const chosen = visible.filter((r) => selected.has(r.project.id));
  const chosenBytes = chosen.reduce((n, r) => n + r.bytes, 0);
  const allVisible = visible.length > 0 && visible.every((r) => selected.has(r.project.id));
  const toggle = (set: (f: (s: Set<number>) => Set<number>) => void, id: number) =>
    set((s) => { const n = new Set(s); if (n.has(id)) n.delete(id); else n.add(id); return n; });

  if (!scanned) {
    return <Empty icon={<FolderOpen className="h-10 w-10 text-primary" />} title="Nothing scanned yet"
      text="Scan a drive or a folder with your projects. DiskSieve finds build artifacts and dependency folders that the project's own tools can recreate."
      action={<Button size="sm" onClick={onScan}><FolderOpen className="h-4 w-4" />Choose what to scan…</Button>} />;
  }
  if (error) return <Empty icon={<Info className="h-10 w-10 text-warning" />} title="Could not load Dev Junk" text={error} />;
  if (!data) return <Empty title="Loading…" />;
  if (!projects.length) {
    return <Empty icon={<Sparkles className="h-10 w-10 text-primary" />} title="No rebuildable artifacts in this scan"
      text="A folder counts only when its project file sits next to it — node_modules beside package.json, target/ beside Cargo.toml, and so on for 25 kinds of artifacts. Installed software (AppData, Program Files, editor extensions, global packages) is never listed." />;
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden p-3 md:gap-4 md:p-5">
      <div className="flex flex-wrap items-center justify-between gap-4 rounded-lg border border-border bg-card p-4 md:p-5">
        <div className="min-w-0">
          <div className="flex items-center gap-2 text-xs uppercase tracking-wider text-muted-foreground"><Sparkles className="h-3.5 w-3.5 text-primary" />Reclaimable developer artifacts</div>
          <div className="mt-1 text-xl font-semibold md:text-2xl">
            <span className="font-mono tabular-nums text-primary">{formatBytes(data.allocatedBytes)}</span>{" "}
            <span className="text-base font-normal text-muted-foreground">in <span className="font-mono tabular-nums">{data.artifactCount.toLocaleString()}</span> {data.artifactCount === 1 ? "artifact" : "artifacts"} across <span className="font-mono tabular-nums">{projects.length.toLocaleString()}</span> {projects.length === 1 ? "project" : "projects"}</span>
          </div>
          <div className="mt-3 flex flex-wrap gap-1.5">
            <Chip active={filter === ALL} onClick={() => setFilter(ALL)}>All</Chip>
            <Chip active={filter === COLD} onClick={() => setFilter(COLD)}>Untouched &gt; {COLD_DAYS} days</Chip>
            {kinds.map(([kind, bytes]) => (
              <Chip key={kind} active={filter === kind} onClick={() => setFilter(kind)}>{junkLabel(kind)} <span className="font-mono tabular-nums opacity-70">{formatBytes(bytes)}</span></Chip>
            ))}
          </div>
        </div>
        <Button disabled={!chosen.length} onClick={() => onTrash(chosen.flatMap((r) => r.artifacts.map(toTarget)))} className="shadow-glow">
          <Trash2 className="h-4 w-4" />Move Selected to Trash <span className="font-mono tabular-nums">({formatBytes(chosenBytes)})</span>
        </Button>
      </div>

      <div className="scrollbar-hover min-h-0 flex-1 overflow-auto rounded-lg border border-border bg-card">
        <Table>
          <TableHeader className="sticky top-0 z-10 bg-card">
            <TableRow>
              <TableHead className="w-10">
                <Checkbox checked={allVisible} aria-label="Select all projects"
                  onCheckedChange={(v) => setSelected((s) => { const n = new Set(s); visible.forEach((r) => (v ? n.add(r.project.id) : n.delete(r.project.id))); return n; })} />
              </TableHead>
              <TableHead>Project</TableHead>
              <TableHead className="hidden lg:table-cell">Artifacts</TableHead>
              <TableHead>Last Code Edit</TableHead>
              <TableHead className="text-right">Size</TableHead>
              <TableHead className="w-20" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {visible.length === 0 && (
              <TableRow><TableCell colSpan={6} className="py-8 text-center text-sm text-muted-foreground">No projects match this filter.</TableCell></TableRow>
            )}
            {visible.map(({ project: p, artifacts, bytes }) => {
              const open = expanded.has(p.id);
              const byKind = new Map<string, number>();
              artifacts.forEach((a) => byKind.set(a.kind, (byKind.get(a.kind) ?? 0) + 1));
              return (
                <Fragment key={p.id}>
                  <TableRow data-state={selected.has(p.id) ? "selected" : undefined}>
                    <TableCell><Checkbox checked={selected.has(p.id)} onCheckedChange={() => toggle(setSelected, p.id)} aria-label={`Select ${p.name}`} /></TableCell>
                    <TableCell className="max-w-0">
                      <button className="flex w-full min-w-0 items-center gap-1.5 text-left" onClick={() => toggle(setExpanded, p.id)} aria-expanded={open}>
                        <ChevronRight className={cn("h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")} />
                        <span className="min-w-0">
                          <span className="block truncate font-medium">{p.name}</span>
                          <span className="block truncate font-mono text-[11px] text-muted-foreground" title={p.path}>{p.path}</span>
                        </span>
                      </button>
                    </TableCell>
                    <TableCell className="hidden lg:table-cell">
                      <div className="flex flex-wrap gap-1">
                        {Array.from(byKind.entries()).map(([kind, count]) => (
                          <Badge key={kind} variant="outline" className="border-primary/40 font-mono text-[10px] text-primary">{junkLabel(kind)}{count > 1 && <span className="ml-1 tabular-nums opacity-70">×{count}</span>}</Badge>
                        ))}
                      </div>
                    </TableCell>
                    <TableCell className={cn("whitespace-nowrap font-mono text-xs tabular-nums", isCold(p) ? "text-warning" : "text-muted-foreground")}>{formatMtime(p.newestMtime)}</TableCell>
                    <TableCell className="text-right font-mono tabular-nums">{formatBytes(bytes)}</TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-0.5">
                        <IconButton label={revealLabel} onClick={() => onReveal(p.id, p.path)}><ExternalLink className="h-3.5 w-3.5" /></IconButton>
                        <IconButton label="Move this project's artifacts to Trash" danger onClick={() => onTrash(artifacts.map(toTarget))}><Trash2 className="h-3.5 w-3.5" /></IconButton>
                      </div>
                    </TableCell>
                  </TableRow>
                  {open && artifacts.map((a) => (
                    <TableRow key={a.id} className="bg-background/40 hover:bg-background/60">
                      <TableCell />
                      <TableCell className="max-w-0 pl-9">
                        <span className="block truncate font-mono text-xs" title={a.path}>{a.relPath}</span>
                      </TableCell>
                      <TableCell className="hidden lg:table-cell"><Badge variant="outline" className="border-primary/40 font-mono text-[10px] text-primary">{junkLabel(a.kind)}</Badge></TableCell>
                      <TableCell className="whitespace-nowrap font-mono text-xs tabular-nums text-muted-foreground">{a.fileCount.toLocaleString()} files</TableCell>
                      <TableCell className="text-right font-mono text-xs tabular-nums">{formatBytes(a.allocatedBytes)}</TableCell>
                      <TableCell>
                        <div className="flex justify-end gap-0.5">
                          <IconButton label={revealLabel} onClick={() => onReveal(a.id, a.path)}><ExternalLink className="h-3.5 w-3.5" /></IconButton>
                          <IconButton label="Move to Trash" danger onClick={() => onTrash([toTarget(a)])}><Trash2 className="h-3.5 w-3.5" /></IconButton>
                        </div>
                      </TableCell>
                    </TableRow>
                  ))}
                </Fragment>
              );
            })}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button onClick={onClick} aria-pressed={active}
      className={cn("flex items-center gap-1.5 rounded-full border px-3 py-1 text-xs transition-colors", active ? "border-primary/60 bg-primary/15 text-primary" : "border-border text-muted-foreground hover:text-foreground")}>
      {children}
    </button>
  );
}

function IconButton({ label, danger, onClick, children }: { label: string; danger?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <Tooltip><TooltipTrigger asChild>
      <Button size="icon" variant="ghost" aria-label={label} onClick={onClick}
        className={cn("h-7 w-7 text-muted-foreground", danger ? "hover:text-destructive" : "hover:text-foreground")}>{children}</Button>
    </TooltipTrigger><TooltipContent className="text-xs">{label}</TooltipContent></Tooltip>
  );
}

function Empty({ icon, title, text, action }: { icon?: ReactNode; title: string; text?: string; action?: ReactNode }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
      {icon}
      <p className="text-sm font-medium">{title}</p>
      {text && <p className="max-w-md text-xs text-muted-foreground">{text}</p>}
      {action}
    </div>
  );
}
