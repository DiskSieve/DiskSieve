import type { TreemapNodeDto } from "@/types/ipc";
import { daysSince } from "@/lib/format";

// Data-visualisation palette (canvas needs concrete colors; mirrors chart tokens in styles.css).
export type ColorMode = "extension" | "age" | "junk";

/** Flag used by the browser demo for folders that contain dev artifacts deeper down. */
export const CONTAINS_JUNK_FLAG = "containsJunk";

export const OTHER_COLOR = "#1A1F1D";

export const EXT_CATS = [
  { key: "video", label: "Video & Media", color: "#2DD4BF", exts: ["mp4", "mov", "mkv", "avi", "webm", "mp3", "flac", "wav", "m4a"] },
  { key: "archive", label: "Archives & Images", color: "#F5B83D", exts: ["zip", "7z", "rar", "tar", "gz", "xz", "zst", "iso", "img", "dmg", "vhdx", "vmdk"] },
  { key: "code", label: "Code & Projects", color: "#22C55E", exts: ["md", "ts", "tsx", "js", "rs", "py", "go", "java", "c", "cpp", "h", "json", "toml", "yaml", "safetensors", "pt", "onnx"] },
  { key: "binary", label: "Binaries & Installers", color: "#7C93B8", exts: ["exe", "msi", "dll", "so", "dylib", "bin", "pak", "app", ""] },
  { key: "system", label: "System", color: "#EF6F4F", exts: ["sys"] },
  { key: "folder", label: "Folders", color: "#3E5A4B", exts: [] },
] as const;

export function extColor(n: TreemapNodeDto) {
  if (n.id === null) return OTHER_COLOR;
  if (n.extension === "sys") return EXT_CATS[4].color;
  if (n.isDir) return n.junkKind ? EXT_CATS[2].color : EXT_CATS[5].color;
  const ext = n.extension ?? "";
  return EXT_CATS.find((c) => (c.exts as readonly string[]).includes(ext))?.color ?? EXT_CATS[3].color;
}

export const AGE_STOPS = [
  { label: "< 30d", color: "#22C55E", max: 30 },
  { label: "< 6m", color: "#2BA89A", max: 180 },
  { label: "< 1y", color: "#64748B", max: 365 },
  { label: "< 2y", color: "#E0A13A", max: 730 },
  { label: "> 2y", color: "#F0694A", max: Infinity },
];

/** Cold Data Heatmap: age of the newest file in the subtree. */
export function ageColor(n: TreemapNodeDto, nowMs = Date.now()) {
  if (n.newestMtime <= 0) return OTHER_COLOR;
  const days = daysSince(n.newestMtime, nowMs);
  return (AGE_STOPS.find((s) => days < s.max) ?? AGE_STOPS[4]!).color;
}

export function junkColor(n: TreemapNodeDto) {
  if (n.junkKind) return "#1ED760";
  return n.flags.includes(CONTAINS_JUNK_FLAG) ? "#1F5E3A" : "#232927";
}

export function colorFor(n: TreemapNodeDto, mode: ColorMode, nowMs = Date.now()) {
  if (n.id === null) return OTHER_COLOR;
  return mode === "extension" ? extColor(n) : mode === "age" ? ageColor(n, nowMs) : junkColor(n);
}

export function legendFor(mode: ColorMode) {
  if (mode === "extension") return EXT_CATS.map((c) => ({ label: c.label, color: c.color }));
  if (mode === "age") return AGE_STOPS.map((s) => ({ label: s.label, color: s.color }));
  return [{ label: "Dev artifact", color: "#1ED760" }, { label: "Contains artifacts", color: "#1F5E3A" }, { label: "Other", color: "#232927" }];
}
