import { useEffect, useState } from "react";
import { ChevronRight, FolderOpen, GitCompare, History, Info, ShoppingBasket } from "lucide-react";
import type { ChangeDto, ChangeKindDto, ChangesDto } from "@/types/ipc";
import { getChanges } from "@/lib/ipc";
import { formatBytes } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Chip, Empty, IconButton } from "./DevJunkView";
import { cn } from "@/lib/utils";

const KINDS: { key: ChangeKindDto; label: string }[] = [
  { key: "added", label: "Added" },
  { key: "removed", label: "Removed" },
  { key: "grew", label: "Grew" },
  { key: "shrank", label: "Shrank" },
  { key: "smallFiles", label: "Small files" },
];
const LABEL = Object.fromEntries(KINDS.map((k) => [k.key, k.label])) as Record<ChangeKindDto, string>;

interface Props {
  generation: number | null;
  /** Changes whenever the tree changes (trash / restore), so the list is fetched again. */
  refreshKey: unknown;
  /** Time of the previous scan of this location; `null` when there is none. */
  previousScanAt: number | null;
  onOpenFolder: (folderId: number) => void;
  /** Adds items (node ids) to the Sieve Basket. */
  onSieve: (ids: number[]) => void;
  onScan: () => void;
}

/** `git diff`-style list of everything that changed since the previous scan. */
export function ChangesView({ generation, refreshKey, previousScanAt, onOpenFolder, onSieve, onScan }: Props) {
  const [kind, setKind] = useState<ChangeKindDto | null>(null);
  const [data, setData] = useState<ChangesDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const available = generation !== null && previousScanAt !== null;

  useEffect(() => setKind(null), [generation]);
  useEffect(() => {
    if (!available || generation === null) {
      setData(null);
      return;
    }
    let live = true;
    // After the parent's effects, so the browser demo sees its updated tree.
    Promise.resolve()
      .then(() => getChanges(generation, kind))
      .then((d) => {
        if (!live || d.generation !== generation) return;
        setData(d);
        setError(null);
      })
      .catch((e: Error) => { if (live) setError(e.message); });
    return () => { live = false; };
  }, [available, generation, kind, refreshKey]);

  if (generation === null) {
    return <Empty icon={<FolderOpen className="h-10 w-10 text-primary" />} title="Nothing scanned yet"
      text="Scan a drive or a folder. The next scan of the same place shows here what appeared, grew, shrank or disappeared in between."
      action={<Button size="sm" onClick={onScan}><FolderOpen className="h-4 w-4" />Choose what to scan…</Button>} />;
  }
  if (previousScanAt === null) {
    return <Empty icon={<History className="h-10 w-10 text-primary" />} title="This is the first scan of this location"
      text="DiskSieve saved a small snapshot of it. Scan the same drive or folder again later — this tab will list every change since now." />;
  }
  if (error) return <Empty icon={<Info className="h-10 w-10 text-warning" />} title="Could not load the changes" text={error} />;
  if (!data) return <Empty title="Loading…" />;

  const total = (k: ChangeKindDto) => data.totals.find((t) => t.kind === k) ?? { kind: k, count: 0, bytes: 0 };
  const allCount = data.totals.reduce((a, t) => a + t.count, 0);
  if (allCount === 0) {
    return <Empty icon={<GitCompare className="h-10 w-10 text-primary" />} title="Nothing changed"
      text={`No file or folder of 1 MiB or more changed since the previous scan (${formatScanDate(data.previousScanAt)}).`} />;
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden p-3 md:gap-4 md:p-5">
      <div className="rounded-lg border border-border bg-card p-4 md:p-5">
        <div className="flex items-center gap-2 text-xs uppercase tracking-wider text-muted-foreground">
          <GitCompare className="h-3.5 w-3.5 text-primary" />Changes since <span className="font-mono normal-case tabular-nums">{formatScanDate(data.previousScanAt)}</span>
        </div>
        <div className="mt-1 flex flex-wrap items-baseline gap-x-4 gap-y-1">
          <span className={cn("font-mono text-xl font-semibold tabular-nums md:text-2xl", signColor(data.net))}>{formatBytes(data.net, true)}</span>
          <span className="font-mono text-sm tabular-nums text-primary">{formatBytes(data.grown, true)}</span>
          <span className="font-mono text-sm tabular-nums text-destructive">{formatBytes(-data.shrunk, true)}</span>
          <span className="text-sm text-muted-foreground"><span className="font-mono tabular-nums">{allCount.toLocaleString()}</span> {allCount === 1 ? "change" : "changes"}</span>
        </div>
        <div className="mt-3 flex flex-wrap gap-1.5">
          <Chip active={kind === null} onClick={() => setKind(null)}>All <span className="font-mono tabular-nums opacity-70">{allCount.toLocaleString()}</span></Chip>
          {KINDS.filter((k) => total(k.key).count > 0).map((k) => (
            <Chip key={k.key} active={kind === k.key} onClick={() => setKind(k.key)}>
              {k.label} <span className="font-mono tabular-nums opacity-70">{total(k.key).count.toLocaleString()} · {formatBytes(total(k.key).bytes, true)}</span>
            </Chip>
          ))}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto rounded-lg border border-border bg-card">
        <Table>
          <TableHeader className="sticky top-0 z-10 bg-card">
            <TableRow>
              <TableHead className="w-8" />
              <TableHead className="w-[55%]">Item</TableHead>
              <TableHead className="hidden md:table-cell">Kind</TableHead>
              <TableHead className="hidden text-right lg:table-cell">Before → After</TableHead>
              <TableHead className="text-right">Change</TableHead>
              <TableHead className="w-20" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {data.changes.map((c, i) => <ChangeRow key={`${c.kind}:${c.path}:${i}`} c={c} onOpenFolder={onOpenFolder} onSieve={onSieve} />)}
          </TableBody>
        </Table>
        {data.matching > data.changes.length && (
          <div className="border-t border-border px-4 py-2 text-xs text-muted-foreground">
            Showing the <span className="font-mono tabular-nums">{data.changes.length.toLocaleString()}</span> largest of <span className="font-mono tabular-nums">{data.matching.toLocaleString()}</span> changes.
          </div>
        )}
      </div>
    </div>
  );
}

function ChangeRow({ c, onOpenFolder, onSieve }: { c: ChangeDto; onOpenFolder: (id: number) => void; onSieve: (ids: number[]) => void }) {
  const up = c.bytes > 0;
  return (
    <TableRow className={up ? "bg-primary/[0.04] hover:bg-primary/[0.08]" : "bg-destructive/[0.04] hover:bg-destructive/[0.08]"}>
      <TableCell className={cn("text-center font-mono text-base font-semibold", up ? "text-primary" : "text-destructive")} aria-label={up ? "grew" : "shrank"}>{up ? "+" : "−"}</TableCell>
      <TableCell className="max-w-0">
        <span className={cn("block truncate", c.kind === "removed" && "line-through decoration-destructive/60", c.kind === "smallFiles" && "italic text-muted-foreground")}>
          {c.name}{c.isDir && <span className="text-muted-foreground">/</span>}
        </span>
        <span className="block truncate font-mono text-[11px] text-muted-foreground" title={c.path}>{c.path}</span>
      </TableCell>
      <TableCell className="hidden md:table-cell">
        <Badge variant="outline" className={cn("text-[10px]", up ? "border-primary/40 text-primary" : "border-destructive/40 text-destructive")}>{LABEL[c.kind]}</Badge>
      </TableCell>
      <TableCell className="hidden whitespace-nowrap text-right font-mono text-xs tabular-nums text-muted-foreground lg:table-cell">
        {c.beforeSize !== null ? formatBytes(c.beforeSize) : "—"} → {c.afterSize !== null ? formatBytes(c.afterSize) : "—"}
      </TableCell>
      <TableCell className={cn("whitespace-nowrap text-right font-mono tabular-nums", signColor(c.bytes))}>{formatBytes(c.bytes, true)}</TableCell>
      <TableCell>
        <div className="flex justify-end gap-0.5">
          {c.nodeId !== null && <IconButton label="Add to Sieve" onClick={() => onSieve([c.nodeId!])}><ShoppingBasket className="h-3.5 w-3.5" /></IconButton>}
          <IconButton label="Show in Explorer" onClick={() => onOpenFolder(c.folderId)}><ChevronRight className="h-3.5 w-3.5" /></IconButton>
        </div>
      </TableCell>
    </TableRow>
  );
}

function signColor(bytes: number) {
  return bytes > 0 ? "text-primary" : bytes < 0 ? "text-destructive" : "text-muted-foreground";
}

function formatScanDate(unixSecs: number) {
  return new Date(unixSecs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}
