import type { SafetyLevelDto } from "@/types/ipc";

/** Badge text and colors of a Safety Advisor verdict (Inspector, Sieve Basket). */
export const LEVELS: Record<SafetyLevelDto, { badge: string; box: string; icon: string; pill: string }> = {
  safe: { badge: "Safe to delete", box: "border-primary/40 bg-primary/10", icon: "text-primary", pill: "" },
  review: { badge: "Review first", box: "border-warning/40 bg-warning/10", icon: "text-warning", pill: "bg-warning text-warning-foreground" },
  system: { badge: "Protected", box: "border-destructive/40 bg-destructive/10", icon: "text-destructive", pill: "bg-destructive text-destructive-foreground" },
};
