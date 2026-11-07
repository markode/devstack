/** Cross-platform open helpers (Tauri opener + dialog). Errors are thrown for callers to toast. */

import { openPath, openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { isTauri } from "./api";

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  return String(e);
}

/** Open http(s) in the system browser. */
export async function openInBrowser(url: string): Promise<void> {
  if (!url) throw new Error("No URL to open");
  if (!isTauri()) {
    window.open(url, "_blank", "noopener,noreferrer");
    return;
  }
  try {
    await openUrl(url);
  } catch (e) {
    throw new Error(`Could not open browser: ${errMessage(e)}`);
  }
}

/** Open a folder or file with the OS default handler (Explorer / Finder / app). */
export async function openInFileManager(path: string): Promise<void> {
  if (!path) throw new Error("No path to open");
  if (!isTauri()) {
    throw new Error("Open folder is only available in the desktop app");
  }
  const normalized = normalizeFsPath(path);
  try {
    // revealItemInDir selects the item; for directories openPath is more reliable on Windows.
    await openPath(normalized);
  } catch (e) {
    try {
      await revealItemInDir(normalized);
    } catch (e2) {
      throw new Error(
        `Could not open folder: ${errMessage(e)}; reveal failed: ${errMessage(e2)}`,
      );
    }
  }
}

/** Open a path with the default application (editor / associated app). */
export async function openInEditor(path: string): Promise<void> {
  if (!path) throw new Error("No path to open");
  if (!isTauri()) {
    throw new Error("Open in editor is only available in the desktop app");
  }
  const normalized = normalizeFsPath(path);
  try {
    await openPath(normalized);
  } catch (e) {
    throw new Error(`Could not open in editor: ${errMessage(e)}`);
  }
}

export type BrowseDumpOptions = {
  defaultPath?: string;
  title?: string;
};

/** Native file picker for SQL dumps. Returns null if cancelled. */
export async function browseDatabaseDump(
  opts: BrowseDumpOptions = {},
): Promise<string | null> {
  if (!isTauri()) {
    throw new Error("Browse is only available in the desktop app");
  }
  try {
    const selected = await openDialog({
      multiple: false,
      directory: false,
      title: opts.title ?? "Select database dump",
      defaultPath: opts.defaultPath ? normalizeFsPath(opts.defaultPath) : undefined,
      filters: [
        {
          name: "Database dumps",
          extensions: ["sql", "gz", "zst", "zstd", "zip"],
        },
        { name: "All files", extensions: ["*"] },
      ],
    });
    if (selected == null) return null;
    return typeof selected === "string" ? selected : selected[0] ?? null;
  } catch (e) {
    throw new Error(`File picker failed: ${errMessage(e)}`);
  }
}

/** Normalize Windows paths that may use mixed separators. */
export function normalizeFsPath(path: string): string {
  // Keep UNC and drive letters; opener accepts both / and \ on Windows.
  return path.trim();
}
