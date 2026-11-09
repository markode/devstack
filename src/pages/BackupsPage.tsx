import { useCallback, useEffect, useState } from "react";
import * as client from "../client";
import { BackupList } from "../components/BackupList";
import { humanizeError } from "../portBusy";
import type { BackupSettings, SiteBackup, SiteRecord } from "../types";

export function BackupsPage({ active = true }: { active?: boolean }) {
  const [backups, setBackups] = useState<SiteBackup[]>([]);
  const [sites, setSites] = useState<SiteRecord[]>([]);
  const [siteId, setSiteId] = useState("");
  const [settings, setSettings] = useState<BackupSettings | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const [b, s, cfg] = await Promise.allSettled([
      client.listSiteBackups(),
      client.getSites(),
      client.getBackupSettings(),
    ]);
    if (b.status === "fulfilled") setBackups(b.value);
    else
      setError(
        humanizeError(
          b.reason instanceof Error ? b.reason.message : String(b.reason),
        ),
      );
    if (s.status === "fulfilled") {
      setSites(s.value);
      setSiteId((cur) => cur || s.value[0]?.id || "");
    }
    if (cfg.status === "fulfilled") setSettings(cfg.value);
  }, []);

  useEffect(() => {
    if (active) void load();
  }, [active, load]);

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

  const saveSettings = (patch: Parameters<typeof client.updateBackupSettings>[0]) =>
    run(async () => {
      setSettings(await client.updateBackupSettings(patch));
    });

  return (
    <div className="stack" data-testid="backups-page">
      <div className="section-head">
        <h2>Backups</h2>
        <p>Files and database snapshots for every site.</p>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}

      <div className="panel-card settings-list">
        <div className="settings-row">
          <label>
            Back up a site
            <span className="hint">Files and database in one archive</span>
          </label>
          <div className="row-actions">
            <select
              className="native"
              aria-label="Site to back up"
              value={siteId}
              disabled={busy || sites.length === 0}
              onChange={(e) => setSiteId(e.target.value)}
            >
              {sites.length === 0 ? <option value="">No sites</option> : null}
              {sites.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy || !siteId}
              onClick={() =>
                void run(async () => {
                  await client.createSiteBackup(siteId);
                })
              }
            >
              {busy ? "Working…" : "Back up now"}
            </button>
          </div>
        </div>
        <div className="settings-row">
          <label>
            Automatic backups
            <span className="hint">
              {settings && settings.intervalSecs > 0
                ? `Every ${Math.max(1, Math.round(settings.intervalSecs / 3600))} h`
                : "Off"}
              {` · keep newest ${settings?.retain ?? 10} per site`}
            </span>
          </label>
          <div className="row-actions">
            <input
              type="number"
              min={1}
              style={{ width: 70 }}
              aria-label="Interval hours"
              disabled={!settings || busy || settings.intervalSecs === 0}
              value={
                settings ? Math.max(1, Math.round(settings.intervalSecs / 3600)) : 24
              }
              onChange={(e) =>
                settings &&
                setSettings({
                  ...settings,
                  intervalSecs: Math.max(1, Number(e.target.value) || 1) * 3600,
                })
              }
              onBlur={() =>
                settings &&
                void saveSettings({ intervalSecs: settings.intervalSecs })
              }
            />
            <input
              type="number"
              min={1}
              style={{ width: 70 }}
              aria-label="Keep last"
              disabled={!settings || busy}
              value={settings?.retain ?? 10}
              onChange={(e) =>
                settings &&
                setSettings({
                  ...settings,
                  retain: Math.max(1, Number(e.target.value) || 1),
                })
              }
              onBlur={() =>
                settings && void saveSettings({ retain: settings.retain })
              }
            />
            <button
              type="button"
              role="switch"
              aria-checked={Boolean(settings && settings.intervalSecs > 0)}
              aria-label="Automatic backups"
              className={`toggle ${settings && settings.intervalSecs > 0 ? "on" : ""}`}
              disabled={!settings || busy}
              onClick={() =>
                settings &&
                void saveSettings({
                  intervalSecs: settings.intervalSecs > 0 ? 0 : 86400,
                })
              }
            />
          </div>
        </div>
      </div>

      <div className="panel-card settings-list" style={{ marginTop: 16 }}>
        <div className="settings-row settings-row-head">
          <label>All backups</label>
          <span className="muted">{backups.length}</span>
        </div>
        <BackupList
          backups={backups}
          busy={busy}
          showSite
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
    </div>
  );
}
