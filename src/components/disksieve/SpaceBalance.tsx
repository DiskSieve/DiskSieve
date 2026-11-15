import { EyeOff, ShieldAlert, ShieldCheck } from "lucide-react";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { formatBytes } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { ScanSummaryDto, SpaceBalanceDto } from "@/types/ipc";

/** Below this share of used space the difference is noise (metadata, rounding). */
const NOTICEABLE_RATIO = 0.01;

const hiddenRatio = (v: SpaceBalanceDto) => (v.usedBytes ? v.unaccountedBytes / v.usedBytes : 0);
const pct = (part: number, whole: number) => (whole ? `${((part / whole) * 100).toFixed(1)}%` : "0%");

// ── Tile in the explorer sub-toolbar ─────────────────────────────────────────

interface TileProps {
  summary: ScanSummaryDto;
  onOpen: () => void;
}

/** "Unscanned / hidden" for a whole-volume scan, otherwise the skipped-entries count. */
export function SpaceBalanceTile({ summary, onOpen }: TileProps) {
  const v = summary.volume;
  if (v) {
    const noticeable = hiddenRatio(v) >= NOTICEABLE_RATIO;
    return (
      <button onClick={onOpen}
        className={cn("flex shrink-0 items-center gap-1.5 rounded-full border px-3 py-1 text-[11px] transition-colors",
          noticeable
            ? "border-warning/50 bg-[repeating-linear-gradient(135deg,transparent_0_6px,color-mix(in_oklch,var(--warning)_10%,transparent)_6px_12px)] text-warning hover:bg-warning/10"
            : "border-border bg-card text-muted-foreground hover:text-foreground")}>
        <EyeOff className="h-3.5 w-3.5" />
        <span>Unscanned / hidden</span>
        <span className="font-mono tabular-nums">{formatBytes(v.unaccountedBytes)}</span>
      </button>
    );
  }
  if (summary.skippedEntries === 0) return null;
  return (
    <button onClick={onOpen}>
      <Badge variant="outline" className="border-warning/40 font-mono text-[10px] tabular-nums text-warning hover:bg-warning/10">
        {summary.skippedEntries.toLocaleString()} skipped (Access Denied)
      </Badge>
    </button>
  );
}

// ── Explanation panel ────────────────────────────────────────────────────────

interface PanelProps {
  summary: ScanSummaryDto | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** `windows`, `linux`, `macos` or `browser` (demo). */
  os: string;
}

export function SpaceBalanceSheet({ summary, open, onOpenChange, os }: PanelProps) {
  const v = summary?.volume ?? null;
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="flex w-full flex-col gap-5 overflow-y-auto sm:w-[460px] sm:max-w-[460px]">
        <SheetHeader>
          <SheetTitle>{v ? "Space balance" : "Skipped entries"}</SheetTitle>
          <SheetDescription className="break-all">
            {v ? "What the drive reports as used, compared with what the scan could see."
              : "Entries the scan could not read. Their size is not included in the results."}
            {summary && <span className="mt-1 block font-mono text-xs">{summary.rootPath}</span>}
          </SheetDescription>
        </SheetHeader>

        {summary && v && <Balance v={v} />}
        {summary && v && <Causes summary={summary} os={os} />}
        {summary && <Skipped summary={summary} />}

        <div className="mt-auto flex items-center gap-2 border-t border-border pt-4">
          <Tooltip><TooltipTrigger asChild>
            <span><Button size="sm" variant="outline" disabled><ShieldAlert className="h-4 w-4" />{os === "windows" || os === "browser" ? "Run as administrator" : "Rescan with elevated rights"}</Button></span>
          </TooltipTrigger><TooltipContent className="text-xs">Coming in v0.3.0</TooltipContent></Tooltip>
          <span className="text-[11px] text-muted-foreground">Elevated scans can read protected folders.</span>
        </div>
      </SheetContent>
    </Sheet>
  );
}

function Balance({ v }: { v: SpaceBalanceDto }) {
  const rows: { label: string; bytes: number; share: string; swatch: string; hint?: string }[] = [
    { label: "Found by the scan", bytes: v.scannedBytes, share: pct(v.scannedBytes, v.totalBytes), swatch: "bg-primary" },
    { label: "Unscanned / hidden", bytes: v.unaccountedBytes, share: pct(v.unaccountedBytes, v.totalBytes), swatch: "bg-warning", hint: `${pct(v.unaccountedBytes, v.usedBytes)} of used space` },
    { label: "Free", bytes: v.freeBytes, share: pct(v.freeBytes, v.totalBytes), swatch: "bg-muted-foreground/30" },
  ];
  const width = (b: number) => `${v.totalBytes ? (b / v.totalBytes) * 100 : 0}%`;
  const fullyAccounted = hiddenRatio(v) < NOTICEABLE_RATIO;
  return (
    <section className="space-y-3">
      <div className="flex h-3 overflow-hidden rounded-full bg-muted" role="img"
        aria-label={`Scanned ${formatBytes(v.scannedBytes)}, hidden ${formatBytes(v.unaccountedBytes)}, free ${formatBytes(v.freeBytes)}`}>
        <div className="h-full bg-primary" style={{ width: width(Math.min(v.scannedBytes, v.usedBytes)) }} />
        <div className="h-full bg-[repeating-linear-gradient(135deg,var(--warning)_0_4px,transparent_4px_8px)]" style={{ width: width(v.unaccountedBytes) }} />
      </div>
      <dl className="space-y-1.5">
        {rows.map((r) => (
          <div key={r.label} className="flex items-baseline gap-2 text-sm">
            <span className={cn("h-2.5 w-2.5 shrink-0 translate-y-[1px] rounded-sm", r.swatch)} aria-hidden />
            <dt className="min-w-0 flex-1">{r.label}{r.hint && <span className="ml-2 font-mono text-[11px] tabular-nums text-muted-foreground">{r.hint}</span>}</dt>
            <dd className="font-mono tabular-nums">{formatBytes(r.bytes)}</dd>
            <dd className="w-14 text-right font-mono text-[11px] tabular-nums text-muted-foreground">{r.share}</dd>
          </div>
        ))}
        <div className="flex items-baseline gap-2 border-t border-border pt-1.5 text-xs text-muted-foreground">
          <span className="w-2.5" aria-hidden />
          <dt className="flex-1">Used (reported by the system) of {formatBytes(v.totalBytes)}</dt>
          <dd className="font-mono tabular-nums">{formatBytes(v.usedBytes)}</dd>
          <dd className="w-14" />
        </div>
      </dl>
      {fullyAccounted && (
        <p className="flex items-center gap-2 rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-xs text-primary">
          <ShieldCheck className="h-4 w-4 shrink-0" />The scan accounts for practically all used space.
        </p>
      )}
    </section>
  );
}

function Causes({ summary, os }: { summary: ScanSummaryDto; os: string }) {
  const denied = summary.skippedEntries;
  const causes: [string, string][] = [
    [
      denied > 0 ? `${denied.toLocaleString()} ${denied === 1 ? "entry" : "entries"} without read permission` : "Folders without read permission",
      "Their contents cannot be listed, so their size is unknown. An elevated scan can read most of them.",
    ],
  ];
  if (os === "windows" || os === "browser") {
    causes.push(
      ["Restore points and shadow copies", "Kept by Windows in “System Volume Information”, which even administrators cannot list."],
      ["NTFS metadata", "The MFT, journal and other system files — usually a few GB on large drives."],
      ["Hard links counted more than once", "Until v0.3.0, files with several hard links (e.g. in WinSxS) may be counted twice, which makes the scan look larger and hides part of the difference."],
    );
  } else if (os === "macos") {
    causes.push(
      ["APFS snapshots and purgeable space", "Time Machine local snapshots and other system snapshots are not visible as files."],
      ["Protected system locations", "Without Full Disk Access some folders cannot be read."],
    );
  } else {
    causes.push(
      ["Reserved blocks", "ext4 reserves ~5% of the volume for root; the system reports it as used."],
      ["Filesystem metadata and snapshots", "Journals, inode tables and Btrfs/ZFS snapshots are not visible as files."],
      ["Files deleted while still open", "Space is freed only when the last process closes the file."],
    );
  }
  return (
    <section className="space-y-2">
      <h3 className="text-sm font-medium">Why is some space hidden?</h3>
      <ul className="space-y-2">
        {causes.map(([title, text]) => (
          <li key={title} className="rounded-md border border-border bg-background px-3 py-2">
            <div className="text-xs font-medium">{title}</div>
            <div className="text-[11px] leading-relaxed text-muted-foreground">{text}</div>
          </li>
        ))}
      </ul>
    </section>
  );
}

function Skipped({ summary }: { summary: ScanSummaryDto }) {
  const { skippedEntries: count, skippedSamples: samples } = summary;
  return (
    <section className="space-y-2">
      <h3 className="flex items-baseline gap-2 text-sm font-medium">
        Skipped entries
        <span className="font-mono text-xs tabular-nums text-muted-foreground">{count.toLocaleString()}</span>
      </h3>
      {count === 0 ? (
        <p className="text-xs text-muted-foreground">Every entry could be read.</p>
      ) : (
        <>
          <ul className="max-h-56 overflow-y-auto rounded-md border border-border bg-background py-1">
            {samples.map((p) => (
              <li key={p} className="truncate px-3 py-0.5 font-mono text-[11px] text-muted-foreground" title={p}>{p}</li>
            ))}
          </ul>
          {count > samples.length && (
            <p className="text-[11px] text-muted-foreground">Showing the first {samples.length} of {count.toLocaleString()}.</p>
          )}
        </>
      )}
    </section>
  );
}
