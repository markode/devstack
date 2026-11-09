import { useEffect, useState } from "react";
import * as client from "../client";
import { humanizeError } from "../portBusy";
import { pickConfigArchive, pickConfigSavePath } from "../open";
import type { ConfigImportReport, TldSettings } from "../types";
import { isValidTld, normalizeTld, tldWarnings } from "../v030";

/** General page: default TLD + config export/import. */
export function ConfigTransferSection() {
  const [tld, setTld] = useState("test");
  const [tldInfo, setTldInfo] = useState<TldSettings | null>(null);
  const [withDbs, setWithDbs] = useState(false);
  const [restoreDbs, setRestoreDbs] = useState(false);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<ConfigImportReport | null>(null);

  useEffect(() => {
    void client
      .getTldSettings()
      .then((s) => {
        setTldInfo(s);
        setTld(s.defaultTld);
      })
      .catch(() => {});
  }, []);

  const guard = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      await fn();
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
    }
  };

  const warnings = tldWarnings(tld);

  return (
    <>
      <div className="section-head" style={{ marginTop: 28 }}>
        <h2>Domains &amp; configuration</h2>
        <p>Default TLD for new sites, and a portable backup of your DevStack setup.</p>
      </div>
      <div className="panel-card settings-list" data-testid="config-transfer">
        <div className="settings-row">
          <label>
            Default TLD
            <span className="hint">New sites become name.{normalizeTld(tld) || "test"}</span>
          </label>
          <div className="row-actions">
            <input aria-label="Default TLD" value={tld} style={{ width: 120 }} onChange={(e) => setTld(e.target.value)} />
            <button
              type="button"
              className="btn btn-install"
              disabled={busy || !isValidTld(tld) || normalizeTld(tld) === tldInfo?.defaultTld}
              onClick={() =>
                void guard(async () => {
                  const s = await client.setDefaultTld(normalizeTld(tld));
                  setTldInfo(s);
                  setNote(`Default TLD set to .${s.defaultTld}`);
                })
              }
            >
              Save
            </button>
          </div>
        </div>
        {warnings.length > 0 ? (
          <div className="settings-row settings-row-block" data-testid="default-tld-warnings">
            {warnings.map((w) => (
              <span key={w} className="error-inline">
                {w}
              </span>
            ))}
          </div>
        ) : null}
        <div className="settings-row">
          <label>
            Export configuration
            <span className="hint">Sites, settings, per-site php.ini and hosting profiles (never passwords).</span>
          </label>
          <div className="row-actions">
            <label className="chip chip-toggle">
              <input type="checkbox" checked={withDbs} onChange={(e) => setWithDbs(e.target.checked)} /> Include databases
            </label>
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy}
              onClick={() =>
                void guard(async () => {
                  const dest = await pickConfigSavePath();
                  if (!dest) return;
                  const r = await client.exportConfig(dest, withDbs);
                  setNote(
                    `Exported ${r.meta.siteCount} site(s) to ${r.path}` +
                      (r.warnings.length ? ` · ${r.warnings.join("; ")}` : ""),
                  );
                })
              }
            >
              Export…
            </button>
          </div>
        </div>
        <div className="settings-row">
          <label>
            Import configuration
            <span className="hint">Merges sites by id and restores settings; ports are re-checked on launch.</span>
          </label>
          <div className="row-actions">
            <label className="chip chip-toggle">
              <input type="checkbox" checked={restoreDbs} onChange={(e) => setRestoreDbs(e.target.checked)} /> Restore databases
            </label>
            <button
              type="button"
              className="btn btn-install"
              disabled={busy}
              onClick={() =>
                void guard(async () => {
                  const path = await pickConfigArchive();
                  if (!path) return;
                  const meta = await client.inspectConfigArchive(path);
                  if (
                    !window.confirm(
                      `Import ${meta.siteCount} site(s) from DevStack ${meta.appVersion} (${meta.createdAt.slice(0, 10)})? Settings will be replaced.`,
                    )
                  )
                    return;
                  const r = await client.importConfig(path, {
                    restoreSettings: true,
                    restoreDatabases: restoreDbs,
                  });
                  setReport(r);
                  setNote(`Imported: ${r.sitesAdded} added, ${r.sitesUpdated} updated`);
                })
              }
            >
              Import…
            </button>
          </div>
        </div>
        {note ? (
          <div className="settings-row" role="status">
            <span className="hint">{note}</span>
          </div>
        ) : null}
        {error ? (
          <div className="settings-row">
            <span className="error-inline">{error}</span>
          </div>
        ) : null}
        {report ? (
          <div className="settings-row settings-row-block" data-testid="import-report">
            {report.missingPaths.length > 0 ? (
              <span className="hint">Missing folders on this machine: {report.missingPaths.join(", ")}</span>
            ) : null}
            {Object.keys(report.installedPackages).length > 0 ? (
              <span className="hint">
                Install on this machine:{" "}
                {Object.entries(report.installedPackages)
                  .map(([id, v]) => `${id} ${v.join("/")}`)
                  .join(" · ")}
              </span>
            ) : null}
            {report.databasesSkipped.map((d) => (
              <span key={d} className="hint">
                Database skipped: {d}
              </span>
            ))}
            {report.notes.map((n) => (
              <span key={n} className="hint">
                {n}
              </span>
            ))}
          </div>
        ) : null}
      </div>
    </>
  );
}
