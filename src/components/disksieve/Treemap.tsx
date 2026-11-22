import { useEffect, useRef } from "react";
import type { TreemapNodeDto } from "@/types/ipc";
import { formatBytes, formatMtime } from "@/lib/format";
import { colorFor, type ColorMode } from "@/lib/treemap-palette";

type ECharts = import("@/lib/echarts").ECharts;

interface Props {
  /** Name of the directory being shown (for "x% of …" in the tooltip). */
  parentName: string;
  items: TreemapNodeDto[];
  /** Area of each tile. */
  valueOf: (n: TreemapNodeDto) => number;
  mode: ColorMode;
  /** Values are signed growth (Delta Mode). */
  signed: boolean;
  onActivate: (n: TreemapNodeDto) => void;
  onContextTarget: (n: TreemapNodeDto | null) => void;
}

const OTHER_KEY = "__other__";
const keyOf = (n: TreemapNodeDto) => (n.id === null ? OTHER_KEY : String(n.id));

const escapeHtml = (s: string) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

export function otherLabel(n: TreemapNodeDto) {
  return `Other (${(n.otherCount ?? 0).toLocaleString()} ${n.otherCount === 1 ? "item" : "items"})`;
}

export function Treemap({ parentName, items, valueOf, mode, signed, onActivate, onContextTarget }: Props) {
  const el = useRef<HTMLDivElement>(null);
  const chart = useRef<ECharts | null>(null);
  const handlers = useRef({ onActivate, onContextTarget, items });
  handlers.current = { onActivate, onContextTarget, items };

  useEffect(() => {
    let disposed = false;
    let ro: ResizeObserver | undefined;
    import("@/lib/echarts").then(({ echarts: ec }) => {
      if (disposed || !el.current) return;
      const c = ec.init(el.current, undefined, { renderer: "canvas" });
      chart.current = c;
      const byKey = (key: string | undefined) => handlers.current.items.find((n) => keyOf(n) === key);
      c.on("click", (p) => {
        const n = byKey((p.data as { id?: string })?.id);
        if (n && n.id !== null) handlers.current.onActivate(n);
      });
      c.on("contextmenu", (p) => {
        const n = byKey((p.data as { id?: string })?.id);
        handlers.current.onContextTarget(n && n.id !== null ? n : null);
      });
      ro = new ResizeObserver(() => c.resize());
      ro.observe(el.current);
      // The latest props: they may have changed while ECharts was loading.
      renderLatest.current();
    });
    return () => { disposed = true; ro?.disconnect(); chart.current?.dispose(); chart.current = null; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const render = () => {
    const c = chart.current;
    if (!c) return;
    const now = Date.now();
    const total = items.reduce((a, n) => a + Math.max(valueOf(n), 0), 0) || 1;
    const data = items.filter((n) => valueOf(n) > 0).map((n) => ({
      id: keyOf(n),
      name: n.id === null ? otherLabel(n) : n.name,
      value: valueOf(n),
      node: n,
      itemStyle: { color: colorFor(n, mode, now), borderColor: "#0B0D0C" },
    }));
    c.setOption({
      animationDurationUpdate: 450,
      tooltip: {
        backgroundColor: "#121514", borderColor: "#1F2422", padding: [8, 10],
        textStyle: { color: "#E8EDEA", fontFamily: "Geist Variable, Geist, sans-serif", fontSize: 12 },
        formatter: (p: { data: { node: TreemapNodeDto; name: string; value: number } }) => {
          const n = p.data.node;
          const pct = ((p.data.value / total) * 100).toFixed(1);
          const mono = "font-family:JetBrains Mono Variable,JetBrains Mono,monospace";
          const name = escapeHtml(p.data.name);
          const tag = n.junkKind ? ` <span style="color:#1ED760">· ${escapeHtml(n.junkKind)}</span>` : "";
          return `<div style="min-width:180px"><b>${name}</b>${tag}<br/>
            <span style="${mono}">${formatBytes(p.data.value, signed)}</span> <span style="color:#8A968F">· ${pct}% of ${escapeHtml(parentName)}</span><br/>
            <span style="color:#8A968F;${mono};font-size:11px">${n.fileCount.toLocaleString()} files · ${formatMtime(n.newestMtime)}</span></div>`;
        },
      },
      series: [{
        type: "treemap", data, roam: false, nodeClick: false, width: "100%", height: "100%", top: 0, left: 0, right: 0, bottom: 0,
        breadcrumb: { show: false }, squareRatio: 0.9,
        label: {
          show: true, position: "insideTopLeft", padding: 8, overflow: "truncate",
          // `{` / `}` would break ECharts rich-text markup; the tooltip shows the full name.
          formatter: (p: { name: string; value: number }) => `{n|${p.name.replace(/[{}]/g, "")}}\n{s|${formatBytes(p.value, signed)}}`,
          rich: {
            n: { color: "#F2F5F3", fontSize: 12, fontWeight: 600, fontFamily: "Geist Variable, Geist, sans-serif", lineHeight: 16 },
            s: { color: "rgba(242,245,243,0.7)", fontSize: 11, fontFamily: "JetBrains Mono Variable, JetBrains Mono, monospace" },
          },
        },
        itemStyle: { borderWidth: 2, gapWidth: 2, borderRadius: 4 },
        emphasis: { itemStyle: { borderColor: "#22C55E", borderWidth: 2 } },
      }],
    }, { replaceMerge: ["series"] });
  };

  const renderLatest = useRef(render);
  renderLatest.current = render;
  useEffect(render);

  return (
    <div className="h-full w-full" onContextMenuCapture={() => onContextTarget(null)}>
      <div ref={el} className="h-full w-full" />
    </div>
  );
}
