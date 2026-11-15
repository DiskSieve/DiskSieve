import { useEffect, useRef, useState, type ReactNode } from "react";
import { File, Folder, Loader2 } from "lucide-react";
import { CommandDialog, CommandGroup, CommandInput, CommandItem, CommandList, CommandShortcut } from "@/components/ui/command";
import { DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { searchNodes, SEARCH_LIMIT } from "@/lib/ipc";
import { formatBytes } from "@/lib/format";
import type { SearchHitDto, SearchResultDto, SizeMetricDto } from "@/types/ipc";

/** Delay between the last keystroke and the search request. */
const DEBOUNCE_MS = 120;

export interface PaletteAction {
  label: string;
  run: () => void;
}

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Current scan; `null` before the first scan (only actions are offered). */
  generation: number | null;
  metric: SizeMetricDto;
  /** Bumped when the tree changes in place (trash / restore), to refresh the hits. */
  treeVersion: number;
  onOpenHit: (hit: SearchHitDto) => void;
  actions: PaletteAction[];
}

const terms = (q: string) => q.toLowerCase().split(/\s+/).filter(Boolean);

/**
 * `Ctrl+K` palette: name search over the scanned tree (in memory, never on disk) — the 50
 * largest matches — plus quick actions filtered by the same words.
 */
export function SearchPalette({ open, onOpenChange, generation, metric, treeVersion, onOpenHit, actions }: Props) {
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<SearchResultDto | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seq = useRef(0);

  // Start empty every time the palette opens.
  useEffect(() => {
    if (open) setQuery("");
  }, [open]);

  useEffect(() => {
    const q = query.trim();
    const id = ++seq.current;
    if (!open || generation === null || !q) {
      setResult(null);
      setLoading(false);
      setError(null);
      return;
    }
    setLoading(true);
    const timer = setTimeout(() => {
      searchNodes(generation, q, metric)
        .then((r) => {
          if (id !== seq.current || r.generation !== generation || r.query !== q) return;
          setResult(r);
          setError(null);
        })
        .catch((e: Error) => id === seq.current && setError(e.message))
        .finally(() => id === seq.current && setLoading(false));
    }, DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [open, query, generation, metric, treeVersion]);

  const words = terms(query);
  const shownActions = actions.filter((a) => words.every((w) => a.label.toLowerCase().includes(w)));
  const hits = result?.hits ?? [];
  const searching = generation !== null && words.length > 0;

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange} shouldFilter={false}>
      <DialogTitle className="sr-only">Search</DialogTitle>
      <DialogDescription className="sr-only">Search files and folders of the current scan by name, or run an action.</DialogDescription>
      <CommandInput value={query} onValueChange={setQuery}
        placeholder={generation === null ? "Search actions… (scan a drive to search files)" : "Search files and folders by name…"} />
      <CommandList>
        {searching && (
          <div className="flex items-center justify-between px-3 pt-2 text-[11px] text-muted-foreground">
            <span>
              {error ? <span className="text-destructive">{error}</span>
                : result ? (result.totalMatches === 0 ? "No matching files or folders"
                  : <>Showing <Num>{hits.length}</Num> largest of <Num>{result.totalMatches}</Num> {result.totalMatches === 1 ? "match" : "matches"}</>)
                : "Searching…"}
            </span>
            {loading && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-label="Searching" />}
          </div>
        )}
        {hits.length > 0 && (
          <CommandGroup heading={`Files and folders · top ${SEARCH_LIMIT} by size`}>
            {hits.map((h) => (
              <CommandItem key={h.node.id} value={`hit-${h.node.id}`} onSelect={() => onOpenHit(h)}>
                {h.node.isDir ? <Folder className="shrink-0 text-primary" /> : <File className="shrink-0" />}
                <div className="flex min-w-0 flex-col">
                  <span className="truncate">{highlight(h.node.name, words)}</span>
                  <span className="truncate font-mono text-[10px] text-muted-foreground" title={h.node.path}>{h.node.path}</span>
                </div>
                <CommandShortcut className="font-mono tabular-nums">{formatBytes(h.node.value)}</CommandShortcut>
              </CommandItem>
            ))}
          </CommandGroup>
        )}
        {shownActions.length > 0 && (
          <CommandGroup heading="Actions">
            {shownActions.map((a) => <CommandItem key={a.label} value={`action-${a.label}`} onSelect={a.run}>{a.label}</CommandItem>)}
          </CommandGroup>
        )}
        {!searching && shownActions.length === 0 && (
          <p className="py-6 text-center text-sm text-muted-foreground">{generation === null ? "No matching actions. Scan a drive to search files." : "No results."}</p>
        )}
        {searching && result?.totalMatches === 0 && shownActions.length === 0 && (
          <p className="py-6 text-center text-sm text-muted-foreground">Nothing named like that in this scan.</p>
        )}
      </CommandList>
    </CommandDialog>
  );
}

const Num = ({ children }: { children: ReactNode }) => <span className="font-mono tabular-nums text-foreground">{children}</span>;

/** Wraps every occurrence of the query words in `<mark>`. */
function highlight(name: string, words: string[]): ReactNode {
  if (!words.length) return name;
  const escaped = words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const parts = name.split(new RegExp(`(${escaped.join("|")})`, "gi"));
  return parts.map((p, i) => (i % 2 === 1 ? <mark key={i} className="rounded-sm bg-primary/20 text-foreground">{p}</mark> : p));
}
