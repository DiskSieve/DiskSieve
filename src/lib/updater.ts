// App updates through `tauri-plugin-updater`.
//
// Rust fetches `latest.json` from GitHub Releases and verifies the minisign signature of the
// package before installing it; the webview never talks to the network (CSP stays `'self'`).
// In a plain browser there is nothing to update, so every function is a no-op there.
import { check, type DownloadEvent } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { isTauri } from "@/lib/ipc";

export interface UpdateProgress {
  downloaded: number;
  /** `null` when the server sends no Content-Length. */
  total: number | null;
}

export interface PendingUpdate {
  version: string;
  currentVersion: string;
  /** Release notes (Markdown from the GitHub release body). */
  notes: string | null;
  date: string | null;
  /** Downloads, verifies and installs the update, then restarts the app. */
  install(onProgress: (p: UpdateProgress) => void): Promise<void>;
}

const AUTO_CHECK_KEY = "disksieve.updates.autoCheck";

/** Whether to look for an update on startup (default on). Kept in `localStorage` until settings land in v0.4.0. */
export function getAutoCheck(): boolean {
  try {
    return localStorage.getItem(AUTO_CHECK_KEY) !== "false";
  } catch {
    return true;
  }
}

export function setAutoCheck(on: boolean): void {
  try {
    localStorage.setItem(AUTO_CHECK_KEY, String(on));
  } catch {
    /* storage unavailable: the choice lasts for this session only */
  }
}

/** `?demo-update` in the dev server fakes an available update so the UI can be checked in a browser. */
const demoUpdate = (): boolean =>
  import.meta.env.DEV && typeof location !== "undefined" && new URLSearchParams(location.search).has("demo-update");

export const updatesSupported = (): boolean => isTauri() || demoUpdate();

/** Asks the release server for a newer version; `null` when this one is current. */
export async function checkForUpdate(): Promise<PendingUpdate | null> {
  if (isTauri()) {
    const update = await check();
    if (!update) return null;
    return {
      version: update.version,
      currentVersion: update.currentVersion,
      notes: update.body?.trim() || null,
      date: update.date ?? null,
      async install(onProgress) {
        let downloaded = 0;
        let total: number | null = null;
        await update.downloadAndInstall((e: DownloadEvent) => {
          if (e.event === "Started") total = e.data.contentLength ?? null;
          else if (e.event === "Progress") downloaded += e.data.chunkLength;
          onProgress({ downloaded, total });
        });
        // Windows exits by itself when the installer starts; macOS and Linux need a restart.
        await relaunch();
      },
    };
  }
  if (demoUpdate()) return fakeUpdate();
  return null;
}

function fakeUpdate(): PendingUpdate {
  return {
    version: "0.2.0",
    currentVersion: "0.1.0",
    notes: "Safety Advisor, the Sieve basket and Delta mode.\n\nThis is a demo update: nothing is downloaded.",
    date: new Date().toISOString(),
    async install(onProgress) {
      const total = 9_400_000;
      for (let downloaded = 0; downloaded <= total; downloaded += 470_000) {
        onProgress({ downloaded, total });
        await new Promise((r) => setTimeout(r, 120));
      }
      throw new Error("Demo mode: the update was not installed.");
    },
  };
}
