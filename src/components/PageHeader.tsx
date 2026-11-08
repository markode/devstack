import type { ReactNode } from "react";

/**
 * Herd-style chrome: thin title bar with optional back arrows, centered title,
 * trailing meta (counts / status), and an optional indeterminate refresh bar.
 */
export function PageHeader({
  title,
  meta,
  refreshing = false,
  showNavArrows = true,
}: {
  title: string;
  meta?: ReactNode;
  refreshing?: boolean;
  showNavArrows?: boolean;
}) {
  return (
    <header className="content-header" data-testid="page-header">
      {showNavArrows ? (
        <div className="nav-arrows" aria-hidden>
          ‹ ›
        </div>
      ) : null}
      <div className="content-title" data-testid="page-header-title">
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
    </header>
  );
}
