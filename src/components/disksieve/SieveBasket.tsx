import type { ReactNode } from "react";
import { File, Folder, ShieldAlert, ShieldCheck, ShoppingBasket, Trash2, X } from "lucide-react";
import type { SieveDto, SieveItemDto, TreemapNodeDto } from "@/types/ipc";
import { LEVELS } from "@/lib/advice";
import { formatBytes } from "@/lib/format";
import type { TrashTarget } from "@/lib/trash";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { cn } from "@/lib/utils";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Number of collected ids (the review may still be loading). */
  count: number;
  sieve: SieveDto | null;
  error: string | null;
  onRemove: (id: number) => void;
  onClear: () => void;
  onInspect: (node: TreemapNodeDto) => void;
  onTrash: (targets: TrashTarget[]) => void;
}

/** What the confirmation lists: the size each item really frees (hard links counted once). */
const toTarget = (i: SieveItemDto): TrashTarget => ({ ...i.node, allocatedSize: i.node.value });

/**
 * Sieve Basket: items collected from the treemap and Dev Junk, reviewed by the backend
 * (`review_sieve`) and moved to Trash together with one confirmation.
 */
export function SieveBasket({ open, onOpenChange, count, sieve, error, onRemove, onClear, onInspect, onTrash }: Props) {
  const items = sieve?.items ?? [];
  const ready = items.filter((i) => i.status === "ready");
  const covered = items.filter((i) => i.status === "covered").length;
  const names = new Map(items.map((i) => [i.node.id, i.node.name]));
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="flex w-full flex-col gap-4 sm:w-[460px] sm:max-w-[460px]">
        <SheetHeader>
          <SheetTitle className="flex items-center gap-2"><ShoppingBasket className="h-5 w-5 text-primary" />Sieve</SheetTitle>
          <SheetDescription>Collect items from the treemap and Dev Junk, check the verdicts, then move them to Trash in one go.</SheetDescription>
        </SheetHeader>

        {count === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 text-center">
            <ShoppingBasket className="h-10 w-10 text-muted-foreground" />
            <p className="text-sm font-medium">The Sieve is empty</p>
            <p className="max-w-xs text-xs text-muted-foreground">Right-click a tile or a list row and choose <span className="text-foreground">Add to Sieve</span>, or add projects from the Dev Junk tab.</p>
          </div>
        ) : (
          <>
            <div className="rounded-lg border border-border bg-background p-4">
              <div className="text-xs text-muted-foreground">Frees when moved to Trash and the Trash is emptied</div>
              <div className="mt-1 font-mono text-2xl tabular-nums">{sieve ? formatBytes(sieve.allocatedBytes) : "—"}</div>
              <div className="mt-2 flex flex-wrap items-center gap-2 font-mono text-[11px] tabular-nums text-muted-foreground">
                <span>{(sieve?.fileCount ?? 0).toLocaleString()} files</span>
                {sieve && sieve.safeCount > 0 && <Badge className="h-5 px-1.5 text-[10px]">{sieve.safeCount} safe</Badge>}
                {sieve && sieve.reviewCount > 0 && <Badge className={cn("h-5 px-1.5 text-[10px]", LEVELS.review.pill)}>{sieve.reviewCount} review first</Badge>}
                {sieve && sieve.protectedCount > 0 && <Badge className={cn("h-5 px-1.5 text-[10px]", LEVELS.system.pill)}>{sieve.protectedCount} protected</Badge>}
              </div>
            </div>

            {error && <p className="text-xs text-destructive">Could not check the Sieve: {error}</p>}
            {sieve && sieve.reviewCount > 0 && (
              <Warning tone="review">{sieve.reviewCount === 1 ? "1 item has" : `${sieve.reviewCount} items have`} no rule saying it is safe. Check the items marked <b>Review first</b> before moving them.</Warning>
            )}
            {sieve && sieve.protectedCount > 0 && (
              <Warning tone="system">{sieve.protectedCount === 1 ? "1 protected item" : `${sieve.protectedCount} protected items`} will be skipped — DiskSieve never moves them to Trash.</Warning>
            )}
            {covered > 0 && (
              <p className="text-xs text-muted-foreground">{covered === 1 ? "1 item is" : `${covered} items are`} inside another collected folder and go to Trash with it (counted once).</p>
            )}

            <div className="-mx-6 min-h-0 flex-1 overflow-y-auto border-y border-border">
              {items.map((i) => {
                const ui = i.status === "covered" ? null : LEVELS[i.advice.level];
                const id = i.node.id!;
                return (
                  <div key={id} className="group flex items-center gap-2.5 border-b border-border/60 px-6 py-2 hover:bg-accent/40">
                    <button className="flex min-w-0 flex-1 items-center gap-2.5 text-left" onClick={() => onInspect(i.node)} title={i.node.path}>
                      {i.node.isDir ? <Folder className="h-4 w-4 shrink-0 text-primary" /> : <File className="h-4 w-4 shrink-0 text-muted-foreground" />}
                      <span className="min-w-0 flex-1">
                        <span className={cn("block truncate text-sm", i.status !== "ready" && "text-muted-foreground")}>{i.node.name}</span>
                        <span className="block truncate text-[11px] text-muted-foreground">
                          {i.status === "covered" ? `Inside “${names.get(i.coveredBy) ?? "a collected folder"}”` : i.status === "protected" ? i.advice.protectedReason : i.advice.title}
                        </span>
                      </span>
                      {ui ? <Badge className={cn("h-5 shrink-0 px-1.5 text-[10px]", ui.pill)}>{ui.badge}</Badge>
                        : <Badge variant="outline" className="h-5 shrink-0 px-1.5 text-[10px] text-muted-foreground">Included</Badge>}
                      <span className={cn("w-16 shrink-0 text-right font-mono text-xs tabular-nums", i.status !== "ready" && "text-muted-foreground line-through")}>{formatBytes(i.node.value)}</span>
                    </button>
                    <Button size="icon" variant="ghost" className="h-7 w-7 shrink-0 text-muted-foreground hover:text-foreground" aria-label={`Remove ${i.node.name} from Sieve`} onClick={() => onRemove(id)}><X className="h-3.5 w-3.5" /></Button>
                  </div>
                );
              })}
              {!sieve && !error && <p className="px-6 py-3 text-xs text-muted-foreground">Checking…</p>}
            </div>

            <div className="flex gap-2">
              <Button variant="outline" onClick={onClear}>Clear</Button>
              <Button variant="destructive" className="flex-1" disabled={!ready.length} onClick={() => onTrash(ready.map(toTarget))}>
                <Trash2 className="h-4 w-4" />Move {ready.length === 1 ? "1 item" : `${ready.length} items`} to Trash…
              </Button>
            </div>
          </>
        )}
      </SheetContent>
    </Sheet>
  );
}

function Warning({ tone, children }: { tone: "review" | "system"; children: ReactNode }) {
  const ui = LEVELS[tone];
  return (
    <div className={cn("flex gap-2 rounded-md border p-3 text-xs", ui.box)}>
      {tone === "system" ? <ShieldAlert className={cn("h-4 w-4 shrink-0", ui.icon)} /> : <ShieldCheck className={cn("h-4 w-4 shrink-0", ui.icon)} />}
      <p className="text-foreground/90">{children}</p>
    </div>
  );
}
