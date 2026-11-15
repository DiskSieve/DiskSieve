import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowUpCircle, Loader2, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Progress } from "@/components/ui/progress";
import { Switch } from "@/components/ui/switch";
import { formatBytes } from "@/lib/format";
import { checkForUpdate, getAutoCheck, setAutoCheck, updatesSupported, type PendingUpdate, type UpdateProgress } from "@/lib/updater";
import type { AppInfoDto } from "@/types/ipc";

/** Startup check waits a moment so it never competes with the first paint or the drive list. */
const STARTUP_CHECK_DELAY_MS = 3000;

export type UpdateStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "upToDate"; at: Date }
  | { kind: "available"; update: PendingUpdate }
  | { kind: "installing"; update: PendingUpdate; progress: UpdateProgress | null }
  | { kind: "error"; message: string };

export interface Updater {
  supported: boolean;
  status: UpdateStatus;
  autoCheck: boolean;
  setAutoCheck: (on: boolean) => void;
  check: () => void;
  install: () => void;
}

export function useUpdater(): Updater {
  const supported = updatesSupported();
  const [status, setStatus] = useState<UpdateStatus>({ kind: "idle" });
  const [autoCheck, setAuto] = useState(getAutoCheck);
  const busy = useRef(false);

  const run = useCallback(async (manual: boolean) => {
    if (!supported || busy.current) return;
    busy.current = true;
    setStatus({ kind: "checking" });
    try {
      const update = await checkForUpdate();
      if (update) {
        setStatus({ kind: "available", update });
      } else {
        setStatus({ kind: "upToDate", at: new Date() });
        if (manual) toast.success("DiskSieve is up to date");
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setStatus({ kind: "error", message });
      // Being offline at startup is normal; only a check the user asked for reports it.
      if (manual) toast.error("Could not check for updates", { description: message });
      else console.warn("update check failed:", message);
    } finally {
      busy.current = false;
    }
  }, [supported]);

  useEffect(() => {
    if (!supported || !getAutoCheck()) return;
    const t = setTimeout(() => void run(false), STARTUP_CHECK_DELAY_MS);
    return () => clearTimeout(t);
  }, [supported, run]);

  const install = useCallback(() => {
    if (status.kind !== "available") return;
    const { update } = status;
    setStatus({ kind: "installing", update, progress: null });
    update
      .install((progress) => setStatus({ kind: "installing", update, progress }))
      .catch((e: unknown) => {
        setStatus({ kind: "available", update });
        toast.error("Update failed", { description: e instanceof Error ? e.message : String(e) });
      });
  }, [status]);

  return {
    supported,
    status,
    autoCheck,
    setAutoCheck: (on) => { setAutoCheck(on); setAuto(on); },
    check: () => void run(true),
    install,
  };
}

const percent = (p: UpdateProgress | null) => (p?.total ? Math.min(100, Math.round((p.downloaded / p.total) * 100)) : null);

/** Discreet pill in the title bar, shown only while an update is available or installing. */
export function UpdateBanner({ updater }: { updater: Updater }) {
  const { status } = updater;
  if (status.kind !== "available" && status.kind !== "installing") return null;
  const { update } = status;
  const installing = status.kind === "installing";
  const pct = installing ? percent(status.progress) : null;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button type="button"
          className="mr-1 flex h-6 items-center gap-1.5 rounded-full border border-primary/40 bg-primary/10 px-2.5 text-[11px] font-medium text-primary transition-colors hover:bg-primary/20">
          {installing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ArrowUpCircle className="h-3.5 w-3.5" />}
          {installing ? <span className="font-mono tabular-nums">Updating{pct !== null ? ` ${pct}%` : "…"}</span> : <>Update <span className="font-mono tabular-nums">v{update.version}</span></>}
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 space-y-3">
        <div>
          <p className="text-sm font-semibold">DiskSieve <span className="font-mono tabular-nums">{update.version}</span> is available</p>
          <p className="text-xs text-muted-foreground">You have <span className="font-mono tabular-nums">{update.currentVersion}</span>. The app restarts after the update.</p>
        </div>
        {update.notes && (
          <div className="max-h-40 overflow-y-auto whitespace-pre-wrap rounded-md border border-border bg-background/60 p-2.5 text-xs text-muted-foreground">{update.notes}</div>
        )}
        {installing && (
          <div className="space-y-1">
            <Progress value={pct ?? 100} className={pct === null ? "animate-pulse" : undefined} />
            <p className="font-mono text-[11px] tabular-nums text-muted-foreground">
              {status.progress ? `${formatBytes(status.progress.downloaded)}${status.progress.total ? ` / ${formatBytes(status.progress.total)}` : ""}` : "Starting download…"}
            </p>
          </div>
        )}
        <Button className="w-full" size="sm" disabled={installing} onClick={updater.install}>
          {installing ? "Installing…" : "Update and restart"}
        </Button>
      </PopoverContent>
    </Popover>
  );
}

/** Version badge that opens app info and the update controls. */
export function VersionMenu({ info, updater }: { info: AppInfoDto | null; updater: Updater }) {
  const { status, supported } = updater;
  const checking = status.kind === "checking";

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button type="button" aria-label="About DiskSieve and updates" className="rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
          <Badge variant="outline" className="h-5 cursor-pointer font-mono text-[10px] text-muted-foreground hover:border-primary/40 hover:text-foreground">
            {info ? `v${info.version}` : "v…"}
          </Badge>
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 space-y-3">
        <div>
          <p className="text-sm font-semibold">DiskSieve <span className="font-mono tabular-nums">{info?.version ?? "…"}</span></p>
          <p className="font-mono text-[11px] text-muted-foreground">{info ? `core ${info.coreVersion} · ${info.os}/${info.arch}` : "loading…"}</p>
        </div>
        <div className="space-y-2 border-t border-border pt-3">
          <div className="flex items-center justify-between gap-2">
            <p className="text-xs text-muted-foreground">{statusText(status, supported)}</p>
            <Button variant="outline" size="sm" className="h-7 shrink-0 text-xs" disabled={!supported || checking || status.kind === "installing"} onClick={updater.check}>
              <RefreshCw className={checking ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} /> Check now
            </Button>
          </div>
          <label className="flex items-center justify-between gap-2 text-xs">
            <span>Check for updates on startup</span>
            <Switch checked={updater.autoCheck} disabled={!supported} onCheckedChange={updater.setAutoCheck} />
          </label>
          <p className="text-[11px] leading-snug text-muted-foreground/80">The only network request DiskSieve makes: it reads the latest version from GitHub Releases. Updates are signature-checked before installing.</p>
        </div>
      </PopoverContent>
    </Popover>
  );
}

function statusText(s: UpdateStatus, supported: boolean): string {
  if (!supported) return "Updates work in the desktop app.";
  switch (s.kind) {
    case "idle": return "Not checked yet.";
    case "checking": return "Checking…";
    case "upToDate": return `Up to date (checked ${s.at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}).`;
    case "available": return `Version ${s.update.version} is available.`;
    case "installing": return `Installing ${s.update.version}…`;
    case "error": return "Last check failed.";
  }
}
