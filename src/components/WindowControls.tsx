import { useEffect, useState } from "react";
import { isTauriRuntime } from "../platform";

/**
 * Windows 11–style caption buttons for undecorated windows.
 * Close hover is red; maximize toggles restore glyph when maximized.
 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let unlisten = () => {};
    void (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        setMaximized(await win.isMaximized());
        unlisten = await win.onResized(async () => {
          try {
            setMaximized(await win.isMaximized());
          } catch {
            /* ignore */
          }
        });
      } catch {
        /* browser / missing ACL */
      }
    })();
    return () => unlisten();
  }, []);

  const run = async (action: "minimize" | "toggleMaximize" | "close") => {
    if (!isTauriRuntime()) return;
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      if (action === "minimize") await win.minimize();
      else if (action === "toggleMaximize") await win.toggleMaximize();
      else await win.close();
    } catch {
      /* ignore */
    }
  };

  return (
    <div className="window-controls" data-testid="window-controls" role="group" aria-label="Window">
      <button
        type="button"
        className="win-btn win-min"
        aria-label="Minimize"
        tabIndex={-1}
        onClick={() => void run("minimize")}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M1 5h8" stroke="currentColor" strokeWidth="1" />
        </svg>
      </button>
      <button
        type="button"
        className="win-btn win-max"
        aria-label={maximized ? "Restore" : "Maximize"}
        tabIndex={-1}
        onClick={() => void run("toggleMaximize")}
      >
        {maximized ? (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
            <path
              d="M2.5 3.5h5v5h-5zM3.5 2.5h5v5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1"
            />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
            <rect
              x="1.5"
              y="1.5"
              width="7"
              height="7"
              fill="none"
              stroke="currentColor"
              strokeWidth="1"
            />
          </svg>
        )}
      </button>
      <button
        type="button"
        className="win-btn win-close"
        aria-label="Close"
        tabIndex={-1}
        onClick={() => void run("close")}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M2 2l6 6M8 2L2 8" stroke="currentColor" strokeWidth="1.1" />
        </svg>
      </button>
    </div>
  );
}
