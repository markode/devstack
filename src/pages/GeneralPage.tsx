import { useEffect, useState } from "react";
import * as client from "../client";
import type { AppInfo, DatabaseFilesSettings, DbFileEntry } from "../types";

export function GeneralPage() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [dbSettings, setDbSettings] = useState<DatabaseFilesSettings | null>(
    null,
  );
  const [dbFolder, setDbFolder] = useState("");
  const [dbFiles, setDbFiles] = useState<DbFileEntry[]>([]);
  const [folderInput, setFolderInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const [i, s, folder, files] = await Promise.all([
        client.getAppInfo(),
        client.getDbFilesSettings(),
        client.getDatabasesFolder(),
        client.listDatabaseFiles(),
      ]);
      setInfo(i);
      setDbSettings(s);
      setDbFolder(folder);
      setFolderInput(s.folder ?? "");
      setDbFiles(files);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const saveDb = async (patch: Parameters<typeof client.updateDbFilesSettings>[0]) => {
    setBusy(true);
    setError(null);
    try {
      const s = await client.updateDbFilesSettings(patch);
      setDbSettings(s);
      setDbFolder(await client.getDatabasesFolder());
      setNote("Databases settings saved");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="stack">
      <div className="hero-card">
        <div
          className="hero-icon"
          style={{
            background: "linear-gradient(145deg, #2f6bff, #0a84ff 55%, #64d2ff)",
          }}
        >
          DS
        </div>
        <h1>DevStack</h1>
        <p>
          Local PHP development environment for macOS and Windows — runtimes,
          databases, and sites in one place.
        </p>
      </div>

      {error ? <div className="error-banner">{error}</div> : null}
      {note ? (
        <div className="muted" style={{ marginBottom: 12, fontSize: 13 }}>
          {note}
        </div>
      ) : null}

      <div className="panel-card settings-list">
        <div className="settings-row">
          <label>
            Version
            <span className="hint">Application build</span>
          </label>
          <span className="muted">{info?.version ?? "…"}</span>
        </div>
        <div className="settings-row">
          <label>
            Platform
            <span className="hint">Detected host</span>
          </label>
          <span className="muted">
            {info ? `${info.os}/${info.arch}` : "…"}
          </span>
        </div>
        <div className="settings-row">
          <label>
            Data directory
            <span className="hint">Packages &amp; state</span>
          </label>
          <span
            className="muted"
            style={{ fontSize: 11, maxWidth: 280, textAlign: "right" }}
          >
            {info?.dataDir ?? "…"}
          </span>
        </div>
        <div className="settings-row">
          <label>
            Preview mode
            <span className="hint">
              {client.useMock()
                ? "UI mock (open via Tauri for live installs)"
                : "Connected to Rust core"}
            </span>
          </label>
          <span className="muted">{client.useMock() ? "Mock" : "Live"}</span>
        </div>
      </div>

      <div className="section-head" style={{ marginTop: 22 }}>
        <h2>Databases folder</h2>
        <p>
          Portable dumps (not raw datadirs) — see{" "}
          <code>docs/databases-folder.md</code>.
        </p>
      </div>
      <div className="panel-card settings-list">
        <div className="settings-row">
          <label>
            Folder
            <span className="hint">{dbFolder || "…"}</span>
          </label>
          <div className="restore-row">
            <input
              value={folderInput}
              placeholder="Leave empty for default"
              onChange={(e) => setFolderInput(e.target.value)}
            />
            <button
              type="button"
              className="btn btn-install"
              disabled={busy}
              onClick={() =>
                void saveDb({ folder: folderInput.trim() || null })
              }
            >
              Save path
            </button>
            <button
              type="button"
              className="btn btn-install"
              onClick={() => {
                void navigator.clipboard?.writeText(dbFolder);
                setNote("Databases path copied");
              }}
            >
              Copy path
            </button>
          </div>
        </div>
        <div className="settings-row">
          <label>
            Auto-export interval
            <span className="hint">Seconds (0 = off)</span>
          </label>
          <input
            type="number"
            min={0}
            style={{ width: 100 }}
            value={dbSettings?.autoExportIntervalSecs ?? 300}
            onChange={(e) =>
              void saveDb({
                autoExportIntervalSecs: Number(e.target.value) || 0,
              })
            }
          />
        </div>
        <div className="settings-row">
          <label>Export on service stop</label>
          <button
            type="button"
            className={`toggle ${dbSettings?.autoExportOnStop ? "on" : ""}`}
            onClick={() =>
              void saveDb({
                autoExportOnStop: !dbSettings?.autoExportOnStop,
              })
            }
          />
        </div>
        <div className="settings-row">
          <label>
            Folder-drop import
            <span className="hint">ask = confirm · auto = import on drop</span>
          </label>
          <select
            className="native"
            value={dbSettings?.importMode ?? "ask"}
            onChange={(e) => void saveDb({ importMode: e.target.value })}
          >
            <option value="ask">Ask before import</option>
            <option value="auto">Auto-import</option>
          </select>
        </div>
        <div className="settings-row">
          <label>
            Export now
            <span className="hint">
              {dbFiles.length
                ? `${dbFiles.length} file(s) on disk`
                : "No dumps yet"}
            </span>
          </label>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy}
            onClick={() =>
              void (async () => {
                setBusy(true);
                setError(null);
                try {
                  const files = await client.exportAllDatabasesToFolder();
                  setDbFiles(await client.listDatabaseFiles());
                  setNote(`Exported ${files.length} database(s)`);
                } catch (e) {
                  setError(e instanceof Error ? e.message : String(e));
                } finally {
                  setBusy(false);
                }
              })()
            }
          >
            Export all running
          </button>
        </div>
      </div>
    </div>
  );
}

export function PlaceholderPage({
  title,
  blurb,
}: {
  title: string;
  blurb: string;
}) {
  return (
    <div className="hero-card">
      <h1>{title}</h1>
      <p>{blurb}</p>
    </div>
  );
}
