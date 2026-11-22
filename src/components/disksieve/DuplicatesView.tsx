import { useEffect, useMemo, useState } from "react";
import { ChevronRight, Copy, FolderOpen, Info, RefreshCw, ScanSearch, ShoppingBasket, X } from "lucide-react";
import type { DuplicateGroupDto, DuplicateProgressDto, DuplicatesDto } from "@/types/ipc";
import { formatBytes, formatMtime } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Progress } from "@/components/ui/progress";
import { Empty, IconButton } from "./DevJunkView";
import { formatDuration } from "./ScanDialogs";
import { cn } from "@/lib/utils";

interface Props {
  generation: number | null;
  /** Result of the last search of this scan; `null` before the first search. */
  data: DuplicatesDto | null;
  searching: boolean;
  progress: DuplicateProgressDto | null;
  /** Node ids already in the Sieve Basket. */
  basket: number[];
  onSearch: () => void;
  onCancel: () => void;
  onOpenFolder: (folderId: number) => void;
  onSieve: (ids: number[]) => void;
  onScan: () => void;
}

/** Duplicate finder: groups of identical files, one copy kept, the rest collected in the Sieve. */
export function DuplicatesView({ generation, data, searching, progress, basket, onSearch, onCancel, onOpenFolder, onSieve, onScan }: Props) {
  // Copy kept per group (by hash); the oldest copy when not chosen.
  const [keep, setKeep] = useState<Record<string, number>>({});
  useEffect(() => setKeep({}), [generation]);
  const inBasket = useMemo(() => new Set(basket), [basket]);

  if (generation === null) {
    return <Empty icon={<FolderOpen className="h-10 w-10 text-primary" />} title="Nothing scanned yet"
      text="Scan a drive or a folder first. DiskSieve then compares files of the same size by content (blake3) to find exact copies."
      action={<Button size="sm" onClick={onScan}><FolderOpen className="h-4 w-4" />Choose what to scan…</Button>} />;
  }
  if (searching) return <SearchProgress progress={progress} onCancel={onCancel} />;
  if (!data) {
    return <Empty icon={<ScanSearch className="h-10 w-10 text-primary" />} title="Find duplicate files"
      text="Files of the same size are compared by their first and last 16 KiB, then by a full blake3 hash. Online-only cloud files, hard links and files under 4 KiB are skipped. This reads file contents, so it can take a while on a large drive."
      action={<Button size="sm" onClick={onSearch} className="shadow-glow"><ScanSearch className="h-4 w-4" />Find duplicates</Button>} />;
  }
  if (data.groupCount === 0) {
    return <Empty icon={<Copy className="h-10 w-10 text-primary" />} title="No duplicates found"
      text={`Checked ${data.filesChecked.toLocaleString()} files of a shared size in ${formatDuration(data.durationMs)}. Nice and tidy.`}
      action={<Button size="sm" variant="outline" onClick={onSearch}><RefreshCw className="h-4 w-4" />Search again</Button>} />;
  }

  const kept = (g: DuplicateGroupDto) => keep[g.hash] ?? g.copies[0].nodeId;
  const extras = (g: DuplicateGroupDto) => g.copies.filter((c) => c.nodeId !== kept(g)).map((c) => c.nodeId);
  const allExtras = data.groups.flatMap(extras);
  const pending = allExtras.filter((id) => !inBasket.has(id));
  const pendingBytes = data.groups.reduce((a, g) => a + extras(g).filter((id) => !inBasket.has(id)).length * g.sizeBytes, 0);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden p-3 md:gap-4 md:p-5">
      <div className="flex flex-wrap items-center justify-between gap-4 rounded-lg border border-border bg-card p-4 md:p-5">
        <div>
          <div className="text-xl font-semibold md:text-2xl">
            <span className="font-mono tabular-nums text-primary">{formatBytes(data.wastedBytes)}</span>{" "}
            <span className="text-base font-normal text-muted-foreground">
              wasted in <span className="font-mono tabular-nums">{data.groupCount.toLocaleString()}</span> {data.groupCount === 1 ? "group" : "groups"} of identical files
            </span>
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            Checked <span className="font-mono tabular-nums">{data.filesChecked.toLocaleString()}</span> files, read <span className="font-mono tabular-nums">{formatBytes(data.bytesRead)}</span> in <span className="font-mono tabular-nums">{formatDuration(data.durationMs)}</span>
            {data.skipped > 0 && <> · <span className="font-mono tabular-nums">{data.skipped.toLocaleString()}</span> unreadable or changed since the scan</>}
            . The oldest copy is kept unless you pick another.
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" onClick={onSearch}><RefreshCw className="h-4 w-4" />Search again</Button>
          <Button disabled={!pending.length} onClick={() => onSieve(pending)} className="shadow-glow">
            <ShoppingBasket className="h-4 w-4" />Add all extra copies to Sieve <span className="font-mono tabular-nums">({formatBytes(pendingBytes)})</span>
          </Button>
        </div>
      </div>

      <div className="min-h-0 flex-1 space-y-3 overflow-auto pr-1">
        {data.groups.map((g) => {
          const groupExtras = extras(g);
          const done = groupExtras.every((id) => inBasket.has(id));
          return (
            <div key={g.hash} className="rounded-lg border border-border bg-card">
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-border px-4 py-3">
                <Copy className="h-4 w-4 shrink-0 text-primary" />
                <span className="min-w-0 truncate font-medium">{g.name}</span>
                <Badge variant="outline" className="font-mono text-[11px] text-muted-foreground" title={`blake3 ${g.hash}`}>b3:{g.hash.slice(0, 12)}</Badge>
                <span className="ml-auto font-mono text-sm tabular-nums">
                  {formatBytes(g.sizeBytes)} <span className="text-muted-foreground">× {g.copies.length}</span>
                  <span className="ml-2 text-primary">{formatBytes(g.wastedBytes)}</span>
                </span>
                <Button size="sm" variant="ghost" className="h-7 text-xs" disabled={done} onClick={() => onSieve(groupExtras)}>
                  <ShoppingBasket className="h-3.5 w-3.5" />{done ? "In Sieve" : "Sieve extras"}
                </Button>
              </div>
              <div role="radiogroup" aria-label={`Copy of ${g.name} to keep`}>
                {g.copies.map((c) => {
                  const isKept = c.nodeId === kept(g);
                  return (
                    <div key={c.nodeId} className={cn("flex items-center gap-3 px-4 py-2 text-sm", isKept ? "bg-primary/[0.05]" : "hover:bg-accent/40")}>
                      <button role="radio" aria-checked={isKept} onClick={() => setKeep((k) => ({ ...k, [g.hash]: c.nodeId }))}
                        className={cn("flex h-4 w-4 shrink-0 items-center justify-center rounded-full border", isKept ? "border-primary" : "border-muted-foreground/50 hover:border-foreground")}
                        aria-label={`Keep ${c.path}`}>
                        {isKept && <span className="h-2 w-2 rounded-full bg-primary" />}
                      </button>
                      <span className={cn("min-w-0 flex-1 truncate font-mono text-xs", isKept ? "text-foreground" : "text-muted-foreground")} title={c.path}>{c.path}</span>
                      {isKept
                        ? <Badge variant="outline" className="shrink-0 border-primary/40 text-[10px] text-primary">Keep</Badge>
                        : inBasket.has(c.nodeId) && <Badge variant="secondary" className="shrink-0 text-[10px]">In Sieve</Badge>}
                      <span className="hidden shrink-0 font-mono text-xs tabular-nums text-muted-foreground sm:inline">{formatMtime(c.mtime)}</span>
                      <IconButton label="Show in Explorer" onClick={() => onOpenFolder(c.folderId)}><ChevronRight className="h-3.5 w-3.5" /></IconButton>
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}
        {data.groupCount > data.groups.length && (
          <p className="px-1 text-xs text-muted-foreground">
            Showing the <span className="font-mono tabular-nums">{data.groups.length.toLocaleString()}</span> groups that waste the most space of <span className="font-mono tabular-nums">{data.groupCount.toLocaleString()}</span>.
          </p>
        )}
      </div>
    </div>
  );
}

const PHASE: Record<DuplicateProgressDto["phase"], { step: string; label: string }> = {
  quick: { step: "1/2", label: "Comparing the first and last 16 KiB of files of the same size" },
  full: { step: "2/2", label: "Hashing the full content of likely duplicates" },
};

function SearchProgress({ progress, onCancel }: { progress: DuplicateProgressDto | null; onCancel: () => void }) {
  const pct = progress && progress.bytesTotal > 0 ? (progress.bytesDone / progress.bytesTotal) * 100 : 0;
  const phase = progress ? PHASE[progress.phase] : null;
  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <div className="w-full max-w-lg rounded-lg border border-border bg-card p-5">
        <div className="flex items-center gap-2 text-sm font-medium">
          <ScanSearch className="h-4 w-4 text-primary" />Finding duplicates…
          {phase && <span className="ml-auto font-mono text-xs tabular-nums text-muted-foreground">step {phase.step}</span>}
        </div>
        <p className="mt-1 text-xs text-muted-foreground">{phase?.label ?? "Collecting files of the same size"}</p>
        <Progress value={pct} className="mt-4 h-2" />
        <div className="mt-2 flex justify-between font-mono text-xs tabular-nums text-muted-foreground">
          <span>{progress ? `${progress.filesDone.toLocaleString()} / ${progress.filesTotal.toLocaleString()} files` : "—"}</span>
          <span>{progress ? `${formatBytes(progress.bytesDone)} / ${formatBytes(progress.bytesTotal)}` : ""}</span>
        </div>
        <div className="mt-4 flex items-center justify-between gap-3">
          <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground"><Info className="h-3 w-3" />Online-only cloud files are never opened.</span>
          <Button size="sm" variant="outline" onClick={onCancel}><X className="h-4 w-4" />Cancel</Button>
        </div>
      </div>
    </div>
  );
}
