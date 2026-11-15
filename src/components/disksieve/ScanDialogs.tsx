import { useEffect, useState } from "react";
import { Cpu, FolderOpen, HardDrive, Loader2, Usb } from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { formatBytes } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { DriveDto, ScanProgressDto } from "@/types/ipc";

export const usedBytes = (d: DriveDto) => Math.max(0, d.totalBytes - d.availableBytes);

export function formatDuration(ms: number) {
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)}s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.round((ms % 60_000) / 1000);
  return m < 60 ? `${m}m ${s}s` : `${Math.floor(m / 60)}h ${m % 60}m`;
}

// ── "What do you want to scan?" ──────────────────────────────────────────────

interface TargetProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  drives: DriveDto[];
  onPickDrive: (mountPoint: string) => void;
  onPickFolder: () => void;
}

export function ScanTargetDialog({ open, onOpenChange, drives, onPickDrive, onPickFolder }: TargetProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl grid-cols-[minmax(0,1fr)] border-border bg-card">
        <DialogHeader>
          <DialogTitle>What do you want to scan?</DialogTitle>
          <DialogDescription>Scan a whole drive or a single folder. DiskSieve only reads file metadata — nothing is opened or changed.</DialogDescription>
        </DialogHeader>
        <div className="grid max-h-[50vh] gap-2 overflow-y-auto scrollbar-hover">
          {drives.length === 0 && <p className="py-6 text-center text-sm text-muted-foreground">Looking for drives…</p>}
          {drives.map((d) => {
            const pct = d.totalBytes ? (usedBytes(d) / d.totalBytes) * 100 : 0;
            const Icon = d.isRemovable ? Usb : HardDrive;
            return (
              <button key={d.mountPoint} onClick={() => onPickDrive(d.mountPoint)}
                className="group flex items-center gap-3 rounded-lg border border-border bg-background p-3 text-left transition-colors hover:border-primary/50 hover:bg-primary/5 focus-visible:border-primary focus-visible:outline-none">
                <Icon className="h-8 w-8 shrink-0 text-muted-foreground group-hover:text-primary" />
                <div className="min-w-0 flex-1">
                  <div className="flex items-baseline gap-2">
                    <span className="font-mono text-sm font-semibold">{d.mountPoint}</span>
                    <span className="truncate text-sm">{d.name || (d.isRemovable ? "Removable Disk" : "Local Disk")}</span>
                    <span className="ml-auto shrink-0 font-mono text-[11px] uppercase text-muted-foreground">{[d.fileSystem, d.kind !== "unknown" ? d.kind : ""].filter(Boolean).join(" · ")}</span>
                  </div>
                  <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted">
                    <div className={cn("h-full rounded-full", pct > 90 ? "bg-destructive" : "bg-primary")} style={{ width: `${pct}%` }} />
                  </div>
                  <div className="mt-1 font-mono text-[11px] tabular-nums text-muted-foreground">
                    {formatBytes(usedBytes(d))} used of {formatBytes(d.totalBytes)} · {formatBytes(d.availableBytes)} free
                  </div>
                </div>
              </button>
            );
          })}
        </div>
        <Button variant="outline" onClick={onPickFolder} className="w-full"><FolderOpen className="h-4 w-4" />Scan a folder…</Button>
      </DialogContent>
    </Dialog>
  );
}

// ── Scan progress (cannot be dismissed; Cancel is the only way out) ──────────

interface ProgressProps {
  path: string | null;
  progress: ScanProgressDto | null;
  /** Used space of the drive being scanned — enables percentage and time remaining. */
  expectedBytes: number | null;
  onCancel: () => void;
}

export function ScanProgressDialog({ path, progress, expectedBytes, onCancel }: ProgressProps) {
  const open = path !== null;
  const [cancelling, setCancelling] = useState(false);
  useEffect(() => { if (open) setCancelling(false); }, [open, path]);

  const elapsed = progress?.elapsedMs ?? 0;
  const secs = elapsed / 1000;
  const bytes = progress?.allocatedBytes ?? 0;
  const files = progress?.files ?? 0;
  const pct = expectedBytes && progress ? Math.min(99, (bytes / expectedBytes) * 100) : null;
  // Bytes found per second is a fair predictor of the rest when scanning a whole drive.
  const remainingMs = pct !== null && pct >= 2 && secs > 1 && bytes > 0
    ? ((expectedBytes! - bytes) / (bytes / secs)) * 1000 : null;

  const stats: [string, string][] = [
    ["Files", files.toLocaleString()],
    ["Folders", (progress?.dirs ?? 0).toLocaleString()],
    ["Size on disk", formatBytes(bytes)],
    ["Files / s", secs > 0.2 ? Math.round(files / secs).toLocaleString() : "—"],
    ["Elapsed", formatDuration(elapsed)],
    ["Remaining", remainingMs !== null ? `~${formatDuration(Math.max(0, remainingMs))}` : "—"],
  ];

  return (
    <Dialog open={open}>
      <DialogContent hideClose className="max-w-lg grid-cols-[minmax(0,1fr)] border-border bg-card"
        onEscapeKeyDown={(e) => e.preventDefault()} onPointerDownOutside={(e) => e.preventDefault()} onInteractOutside={(e) => e.preventDefault()}>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2"><Loader2 className="h-4 w-4 animate-spin text-primary" />Scanning</DialogTitle>
          <DialogDescription className="break-all font-mono text-xs">{path}</DialogDescription>
        </DialogHeader>

        <div className="space-y-1.5">
          {pct !== null ? <Progress value={pct} className="h-2" />
            : <div className="h-2 overflow-hidden rounded-full bg-muted"><div className="h-full w-1/3 animate-[scan-slide_1.2s_ease-in-out_infinite] rounded-full bg-primary" /></div>}
          <div className="flex justify-between font-mono text-[11px] tabular-nums text-muted-foreground">
            <span>{pct !== null ? `${pct.toFixed(1)}%` : "Counting…"}</span>
            {expectedBytes !== null && <span>{formatBytes(bytes)} / ~{formatBytes(expectedBytes)}</span>}
          </div>
        </div>

        <dl className="grid grid-cols-3 gap-2">
          {stats.map(([k, v]) => (
            <div key={k} className="rounded-md border border-border bg-background px-3 py-2">
              <dt className="text-[11px] text-muted-foreground">{k}</dt>
              <dd className="truncate font-mono text-sm tabular-nums">{v}</dd>
            </div>
          ))}
        </dl>

        <div className="rounded-md border border-border bg-background px-3 py-2">
          <div className="text-[11px] text-muted-foreground">Current folder</div>
          <div className="truncate font-mono text-xs" dir="rtl" title={progress?.currentDir}><bdi>{progress?.currentDir || "Starting…"}</bdi></div>
        </div>

        <div className="flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
          <span className="flex items-center gap-1.5"><Cpu className="h-3.5 w-3.5" /><span className="font-mono tabular-nums">{progress?.threads ?? "—"}</span> threads</span>
          {(progress?.skippedEntries ?? 0) > 0 && (
            <span className="text-warning"><span className="font-mono tabular-nums">{progress!.skippedEntries.toLocaleString()}</span> skipped (Access Denied)</span>
          )}
          <Button variant="outline" size="sm" className="ml-auto" disabled={cancelling}
            onClick={() => { setCancelling(true); onCancel(); }}>
            {cancelling ? "Cancelling…" : "Cancel"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
