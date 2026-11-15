import { forwardRef, useEffect, useState, type ButtonHTMLAttributes } from "react";
import { Copy, Minus, Search, Settings, Square, X } from "lucide-react";
import { Logo } from "./Logo";
import { UpdateBanner, VersionMenu, useUpdater } from "./Updates";
import { Badge } from "@/components/ui/badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { isTauri } from "@/lib/ipc";
import { closeWindow, minimizeWindow, toggleMaximizeWindow, watchMaximized } from "@/lib/window";
import type { AppInfoDto } from "@/types/ipc";

interface Props {
  info: AppInfoDto | null;
  onSearch: () => void;
}

/**
 * Custom window title bar (the native one is disabled with `decorations: false`).
 * Empty areas carry `data-tauri-drag-region`: drag to move, double-click to maximize.
 */
export function TitleBar({ info, onSearch }: Props) {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => watchMaximized(setMaximized), []);
  const desktop = isTauri();
  const updater = useUpdater();

  return (
    <header data-tauri-drag-region className="relative z-20 grid h-10 shrink-0 select-none grid-cols-[1fr_minmax(0,420px)_1fr] items-center border-b border-border bg-card">
      <div data-tauri-drag-region className="flex min-w-0 items-center gap-2 pl-3">
        <Logo className="pointer-events-none h-5 w-5" />
        <span data-tauri-drag-region className="text-[13px] font-semibold tracking-tight">DiskSieve</span>
        <VersionMenu info={info} updater={updater} />
        {!desktop && <Badge variant="outline" className="hidden h-5 border-warning/40 text-[10px] text-warning sm:inline-flex">demo data</Badge>}
      </div>

      <button onClick={onSearch} aria-label="Search files"
        className="flex h-7 w-full items-center gap-2 rounded-md border border-border bg-background/60 px-2.5 text-xs text-muted-foreground transition-colors hover:border-primary/40 hover:text-foreground">
        <Search className="h-3.5 w-3.5 shrink-0" />
        <span className="truncate">Search files and folders…</span>
        <kbd className="ml-auto rounded border border-border px-1 font-mono text-[10px]">Ctrl K</kbd>
      </button>

      <div data-tauri-drag-region className="flex h-full items-center justify-end">
        <UpdateBanner updater={updater} />
        <Tooltip><TooltipTrigger asChild>
          <WindowButton label="Settings" onClick={() => {}}><Settings className="h-4 w-4" /></WindowButton>
        </TooltipTrigger><TooltipContent className="text-xs">Settings — coming soon</TooltipContent></Tooltip>
        <span className="mx-1 h-4 w-px bg-border" aria-hidden />
        <WindowButton label="Minimize" onClick={minimizeWindow} disabled={!desktop}><Minus className="h-4 w-4" /></WindowButton>
        <WindowButton label={maximized ? "Restore" : "Maximize"} onClick={toggleMaximizeWindow} disabled={!desktop}>
          {maximized ? <Copy className="h-3.5 w-3.5 -scale-x-100" /> : <Square className="h-3.5 w-3.5" />}
        </WindowButton>
        <WindowButton label="Close" onClick={closeWindow} disabled={!desktop} danger><X className="h-4 w-4" /></WindowButton>
      </div>
    </header>
  );
}

type WindowButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & { label: string; danger?: boolean };

const WindowButton = forwardRef<HTMLButtonElement, WindowButtonProps>(({ label, danger, className, children, ...rest }, ref) => (
  <button ref={ref} type="button" aria-label={label} {...rest}
    className={cn("flex h-10 w-11 items-center justify-center text-muted-foreground transition-colors disabled:pointer-events-none disabled:opacity-40",
      danger ? "hover:bg-[#E81123] hover:text-white" : "hover:bg-accent hover:text-foreground", className)}>
    {children}
  </button>
));
WindowButton.displayName = "WindowButton";
