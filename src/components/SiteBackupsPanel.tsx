import { useCallback, useEffect, useState } from "react";
import * as client from "../client";
import { humanizeError } from "../portBusy";
import type { SiteBackup } from "../types";
import { BackupList } from "./BackupList";

/** Per-site backups card used on the site detail view. */
export function SiteBackupsPanel({ siteId }: { siteId: string }) {
  const [backups, setBackups] = useState<SiteBackup[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setBackups(await client.listSiteBackups(siteId));
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    }
  }, [siteId]);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
      await load();
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="panel-card settings-list"
      style={{ marginTop: 16 }}
      data-testid="site-backups"
    >
      <div className="settings-row settings-row-head">
        <label>Site backups</label>
        <button
          type="button"
          className="btn btn-primary"
          disabled={busy}
          onClick={() =>
            void run(async () => {
              await client.createSiteBackup(siteId);
            })
          }
        >
          {busy ? "Working…" : "Back up now"}
        </button>
      </div>
      {error ? (
        <div className="settings-row">
          <span className="error-inline">{error}</span>
        </div>
      ) : null}
      <BackupList
        backups={backups}
        busy={busy}
        onRestore={(b) => {
          if (
            !window.confirm(
              "Restore this backup? Current files and database are replaced.",
            )
          )
            return;
          void run(async () => {
            await client.restoreSiteBackup(b.id);
          });
        }}
        onDelete={(b) => {
          if (!window.confirm("Delete this backup permanently?")) return;
          void run(() => client.deleteSiteBackup(b.id));
        }}
      />
    </div>
  );
}
