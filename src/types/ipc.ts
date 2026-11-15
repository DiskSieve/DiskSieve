// IPC type contract (Rust ↔ TypeScript).
// DTO types are generated from Rust by ts-rs into ./generated — never edit them by hand.
// Regenerate after changing `src-tauri/src/dto.rs`: `npm run gen:types`.
export type { AppInfoDto } from "./generated/AppInfoDto";
export type { BreadcrumbDto } from "./generated/BreadcrumbDto";
export type { DriveDto } from "./generated/DriveDto";
export type { ScanProgressDto } from "./generated/ScanProgressDto";
export type { ScanSummaryDto } from "./generated/ScanSummaryDto";
export type { SizeMetricDto } from "./generated/SizeMetricDto";
export type { SpaceBalanceDto } from "./generated/SpaceBalanceDto";
export type { TreemapNodeDto } from "./generated/TreemapNodeDto";
export type { ViewportDto } from "./generated/ViewportDto";
