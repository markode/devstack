import type { SiteBackup } from "../types";

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function formatWhen(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

export function BackupList({
  backups,
  busy = false,
  showSite = false,
  onRestore,
  onDelete,
}: {
  backups: SiteBackup[];
  busy?: boolean;
  showSite?: boolean;
  onRestore: (b: SiteBackup) => void;
  onDelete: (b: SiteBackup) => void;
}) {
  if (backups.length === 0) {
    return (
      <div className="settings-row" data-testid="backups-empty">
        <label>
          No backups yet
          <span className="hint">Create one to snapshot files and database.</span>
        </label>
      </div>
    );
  }
  return (
    <>
      {backups.map((b) => (
        <div className="settings-row" key={b.id} data-testid="backup-row">
          <label>
            {showSite ? <strong>{b.siteName || b.siteId}</strong> : null}
            {formatWhen(b.createdAt)}
            <span className="hint">
              {formatBytes(b.sizeBytes)} · files
              {b.dbArchive ? ` · database${b.dbName ? ` (${b.dbName})` : ""}` : ""}
              {b.warnings.length ? ` · ${b.warnings.length} warning(s)` : ""}
            </span>
          </label>
          <div className="row-actions">
            <button
              type="button"
              className="btn btn-install"
              disabled={busy}
              onClick={() => onRestore(b)}
            >
              Restore
            </button>
            <button
              type="button"
              className="btn btn-danger"
              disabled={busy}
              onClick={() => onDelete(b)}
            >
              Delete
            </button>
          </div>
        </div>
      ))}
    </>
  );
}
