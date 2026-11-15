import { Fragment, useLayoutEffect, useRef, useState } from "react";
import { Breadcrumb, BreadcrumbEllipsis, BreadcrumbItem, BreadcrumbLink, BreadcrumbList, BreadcrumbPage, BreadcrumbSeparator } from "@/components/ui/breadcrumb";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import type { BreadcrumbDto } from "@/types/ipc";

/** Rough width budget per visible segment (label + separator), in px. */
const SEGMENT_PX = 96;
/** Names up to this many characters are always shown in full. */
const SHORT_NAME = 5;

interface Props {
  crumbs: BreadcrumbDto[];
  onNavigate: (id: number) => void;
}

/**
 * Path from the scan root to the current folder. It never scrolls: when the
 * segments do not fit, the middle ones collapse into a "…" menu. The current
 * folder always stays visible, then its parent, then the root.
 */
export function PathBreadcrumb({ crumbs, onNavigate }: Props) {
  const ref = useRef<HTMLElement>(null);
  const [width, setWidth] = useState(Number.POSITIVE_INFINITY);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setWidth(el.clientWidth);
    const ro = new ResizeObserver(([entry]) => setWidth(entry!.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // How many segments fit (at least the current folder); the "…" menu takes one slot.
  const fit = Math.max(1, Math.floor(width / SEGMENT_PX));
  const collapse = crumbs.length > fit;
  const slots = collapse ? Math.max(1, fit - 1) : crumbs.length;
  const keepHead = collapse && slots >= 3;
  const tailCount = keepHead ? slots - 1 : slots;
  const head = collapse ? (keepHead ? crumbs.slice(0, 1) : []) : crumbs;
  const tail = collapse ? crumbs.slice(-tailCount) : [];
  const hidden = collapse ? crumbs.slice(head.length, crumbs.length - tailCount) : [];
  const last = crumbs[crumbs.length - 1];

  const crumb = (b: BreadcrumbDto, first: boolean) => (
    <Fragment key={b.id}>
      {!first && <BreadcrumbSeparator />}
      {/* Parents give up width first; the current folder keeps it longest. Short names never
          shrink (a min-width wider than the text would leave a gap after it). */}
      <BreadcrumbItem className={b.name.length <= SHORT_NAME ? "shrink-0" : b === last ? "min-w-[3rem] shrink" : "min-w-[2.5rem] shrink-[8]"}>
        {b === last ? (
          <BreadcrumbPage className="min-w-0 truncate font-mono" title={b.name}>{b.name}</BreadcrumbPage>
        ) : (
          <BreadcrumbLink className="min-w-0 max-w-[12rem] cursor-pointer truncate font-mono" title={b.name} onClick={() => onNavigate(b.id)}>{b.name}</BreadcrumbLink>
        )}
      </BreadcrumbItem>
    </Fragment>
  );

  return (
    <Breadcrumb ref={ref} className="min-w-0 flex-1 overflow-hidden">
      <BreadcrumbList className="flex-nowrap whitespace-nowrap text-xs">
        {head.map((b, i) => crumb(b, i === 0))}
        {hidden.length > 0 && (
          <>
            {head.length > 0 && <BreadcrumbSeparator />}
            <BreadcrumbItem className="shrink-0">
              <DropdownMenu>
                <DropdownMenuTrigger className="flex items-center rounded-sm hover:text-foreground" aria-label={`Show ${hidden.length} hidden folders`}>
                  <BreadcrumbEllipsis className="h-6 w-6" />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start">
                  {hidden.map((b) => (
                    <DropdownMenuItem key={b.id} className="font-mono text-xs" onSelect={() => onNavigate(b.id)}>{b.name}</DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            </BreadcrumbItem>
          </>
        )}
        {tail.map((b, i) => crumb(b, i === 0 && head.length === 0 && hidden.length === 0))}
      </BreadcrumbList>
    </Breadcrumb>
  );
}
