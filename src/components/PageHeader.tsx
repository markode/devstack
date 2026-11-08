import type { ReactNode } from "react";
import { WindowControls } from "./WindowControls";
import { isTauriRuntime, type AppPlatform } from "../platform";

/**
 * Native top chrome: drag region, centered title, optional meta/loader,
 * and Windows caption buttons when undecorated.
 */
export function PageHeader({
  title,
  meta,
  refreshing = false,
  platform = "web",
  showWindowControls = false,
}: {
  title: string;
  meta?: ReactNode;
  refreshing?: boolean;
  platform?: AppPlatform;
  showWindowControls?: boolean;
}) {
  const onDoubleClick = () => {
    if (!isTauriRuntime()) return;
    void (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        await getCurrentWindow().toggleMaximize();
      } catch {
        /* ignore */
      }
    })();
  };

  return (
    <header
      className={`content-header platform-${platform}`}
      data-testid="page-header"
      data-tauri-drag-region
      onDoubleClick={onDoubleClick}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div
        className="content-header-drag"
        data-tauri-drag-region
        aria-hidden
      />
      <div
        className="content-title"
        data-testid="page-header-title"
        data-tauri-drag-region
      >
        {title}
      </div>
      {meta ? (
        <div className="content-header-meta" aria-live="polite">
          {meta}
        </div>
      ) : null}
      {refreshing ? (
        <div
          className="content-header-loader"
          role="status"
          aria-label="Refreshing"
          data-testid="sites-header-loader"
        />
      ) : null}
      {showWindowControls ? <WindowControls /> : null}
    </header>
  );
}
