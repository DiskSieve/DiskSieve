/** Anything that can be offered for "Move to Trash": a treemap tile, a Dev Junk artifact, … */
export interface TrashTarget {
  /** Scan node id; `null` for the aggregated "Other" tile (never trashed). */
  id: number | null;
  name: string;
  path: string;
  allocatedSize: number;
  fileCount: number;
}
