// Window controls for the custom title bar. In a plain browser (demo mode) they are no-ops.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "@/lib/ipc";

const run = (action: (w: ReturnType<typeof getCurrentWindow>) => Promise<unknown>) => {
  if (!isTauri()) return;
  action(getCurrentWindow()).catch((e: unknown) => console.error("window action failed", e));
};

export const minimizeWindow = () => run((w) => w.minimize());
export const toggleMaximizeWindow = () => run((w) => w.toggleMaximize());
export const closeWindow = () => run((w) => w.close());

/** Calls `onChange` with the maximized state now and whenever the window is resized. */
export function watchMaximized(onChange: (maximized: boolean) => void): () => void {
  if (!isTauri()) return () => {};
  const w = getCurrentWindow();
  let disposed = false;
  let unlisten: (() => void) | undefined;
  const update = () => w.isMaximized().then((m) => { if (!disposed) onChange(m); }).catch(() => {});
  update();
  w.onResized(update).then((u) => { if (disposed) u(); else unlisten = u; }).catch(() => {});
  return () => { disposed = true; unlisten?.(); };
}
