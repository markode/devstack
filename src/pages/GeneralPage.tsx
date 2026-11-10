import { useEffect, useState } from "react";
import * as client from "../client";
import { ConfigTransferSection } from "../components/ConfigTransferSection";
import type {
  AppInfo,
  DatabaseFilesSettings,
  DbFileEntry,
  DeveloperSettings,
  HostsStatus,
  PortsConfig,
  ToolsStatus,
} from "../types";
import { browseFolder } from "../open";
import { ParkedDirsSection } from "../components/ParkedDirsSection";

const PORT_FIELDS: {
  key: keyof PortsConfig;
  label: string;
  hint: string;
}[] = [
  { key: "nginxHttp", label: "Nginx HTTP", hint: "Default 80" },
  { key: "nginxHttps", label: "Nginx HTTPS", hint: "Default 443 · phpMyAdmin" },
  { key: "apacheHttp", label: "Apache HTTP", hint: "Default 8088" },
  { key: "apacheHttps", label: "Apache HTTPS", hint: "Default 8444" },
  { key: "mysql", label: "MySQL", hint: "Default 3306 · coexistence 3307" },
  { key: "mariadb", label: "MariaDB", hint: "Default 3306 · coexistence 3308" },
  {
    key: "postgresql",
    label: "PostgreSQL",
    hint: "Default 5432 · coexistence 5433",
  },
  {
    key: "mongodb",
    label: "MongoDB",
    hint: "Default 27017 · coexistence 27018",
  },
  { key: "phpFpm", label: "PHP-FPM base", hint: "Windows TCP base (9000)" },
];

export function GeneralPage({
  onDeveloperChange,
}: {
  onDeveloperChange?: (d: DeveloperSettings) => void;
} = {}) {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [dbSettings, setDbSettings] = useState<DatabaseFilesSettings | null>(
    null,
  );
  const [dbFolder, setDbFolder] = useState("");
  const [dbFiles, setDbFiles] = useState<DbFileEntry[]>([]);
  const [folderInput, setFolderInput] = useState("");
  const [ports, setPorts] = useState<PortsConfig | null>(null);
  const [hosts, setHosts] = useState<HostsStatus | null>(null);
  const [developer, setDeveloper] = useState<DeveloperSettings>({
    showLogsTab: true,
    redactSecrets: false,
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [tools, setTools] = useState<ToolsStatus | null>(null);

  const ensureTools = async () => {
    setBusy(true);
    setError(null);
    try {
      setTools(await client.ensureDevTools());
      setNote("Command-line tools are ready");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const refresh = async () => {
    try {
      const [i, s, folder, files, p, h, d] = await Promise.all([
        client.getAppInfo(),
        client.getDbFilesSettings(),
        client.getDatabasesFolder(),
        client.listDatabaseFiles(),
        client.getPortsConfig(),
        client.getHostsStatus(),
        client.getDeveloperSettings(),
      ]);
      setInfo(i);
      setDbSettings(s);
      setDbFolder(folder);
      setFolderInput(s.folder ?? "");
      setDbFiles(files);
      setPorts(p);
      setHosts(h);
      setDeveloper(d);
      onDeveloperChange?.(d);
      void client.getToolsStatus().then(setTools).catch(() => {});
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const saveDeveloper = async (patch: Partial<DeveloperSettings>) => {
    setBusy(true);
    setError(null);
    try {
      const d = await client.updateDeveloperSettings(patch);
      setDeveloper(d);
      onDeveloperChange?.(d);
      setNote("Developer settings saved");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const repairHosts = async () => {
    setBusy(true);
    setError(null);
    try {
      const h = await client.repairHosts();
      setHosts(h);
      setNote(
        h.ok
          ? "Hosts and CA trust updated — .test sites should resolve locally"
          : (h.message ?? "Hosts repair incomplete"),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
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

  const savePorts = async (next: PortsConfig) => {
    setBusy(true);
    setError(null);
    try {
      const p = await client.updatePortsConfig(next);
      setPorts(p);
      setNote("Ports saved — restart services to bind the new listeners");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const toggleCoexistence = async () => {
    if (!ports) return;
    setBusy(true);
    setError(null);
    try {
      const p = await client.setCoexistenceMode(!ports.coexistenceMode);
      setPorts(p);
      setNote(
        p.coexistenceMode
          ? "Coexistence mode on (8080/8443, MySQL 3307, MariaDB 3308, …)"
          : "Coexistence mode off — classic default ports restored",
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="stack">
      <div className="section-head">
        <h2>General</h2>
        <p>
          Stack preferences, ports, hosts, and developer options for this
          machine.
        </p>
      </div>

      {error ? <div className="error-banner">{error}</div> : null}
      {note ? (
        <div className="muted" style={{ marginBottom: 12, fontSize: 13 }}>
          {note}
        </div>
      ) : null}
      {hosts && !hosts.ok ? (
        <div className="error-banner" style={{ display: "flex", gap: 12, alignItems: "center" }}>
          <span style={{ flex: 1 }}>
            {hosts.message ??
              "Some .test domains are missing from the hosts file."}
          </span>
          <button
            type="button"
            className="btn btn-install"
            disabled={busy}
            onClick={() => void repairHosts()}
          >
            Fix hosts
          </button>
        </div>
      ) : null}
      {hosts && hosts.caTrusted === false ? (
        <div
          className="error-banner"
          data-testid="ca-untrusted-banner"
          style={{ display: "flex", gap: 12, alignItems: "center" }}
        >
          <span style={{ flex: 1 }}>
            {hosts.caMessage ??
              "HTTPS certificate not trusted — click Retry to install the DevStack CA."}
          </span>
          <button
            type="button"
            className="btn btn-install"
            disabled={busy}
            onClick={() => void repairHosts()}
          >
            Retry
          </button>
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

      <ParkedDirsSection
        pickFolder={() => browseFolder({ title: "Park a directory" })}
        onNote={setNote}
        onError={setError}
      />

      <div className="section-head" style={{ marginTop: 22 }}>
        <h2>Command-line tools</h2>
        <p>
          <code>php</code>, <code>composer</code>, <code>wp</code> and{" "}
          <code>node</code> shims for your terminals.
        </p>
      </div>
      <div className="panel-card settings-list" data-testid="tools-status">
        <div className="settings-row">
          <label>
            Tools
            <span className="hint">
              {tools
                ? [
                    ["Composer", tools.composer],
                    ["WP-CLI", tools.wpCli],
                    ["Node", tools.node],
                    ["Shims", tools.shims],
                  ]
                    .map(([name, ok]) => `${name} ${ok ? "ready" : "missing"}`)
                    .join(" · ")
                : "Status unavailable"}
            </span>
          </label>
          <button
            type="button"
            className="btn btn-install"
            disabled={busy}
            onClick={() => void ensureTools()}
          >
            {tools?.composer && tools.wpCli && tools.shims
              ? "Re-check tools"
              : "Install tools"}
          </button>
        </div>
      </div>

      <div className="section-head" style={{ marginTop: 22 }}>
        <h2>Local DNS / hosts</h2>
        <p>
          DevStack maps <code>*.test</code> to 127.0.0.1. On Windows this uses
          one UAC prompt for hosts + CA trust — no manual edits.
        </p>
      </div>
      <div className="panel-card settings-list">
        <div className="settings-row">
          <label>
            Hosts status
            <span className="hint">
              {hosts?.ok
                ? "All configured .test domains resolve locally"
                : hosts?.missing?.length
                  ? `Missing: ${hosts.missing.join(", ")}`
                  : "Checking…"}
            </span>
          </label>
          <button
            type="button"
            className="btn btn-install"
            disabled={busy}
            onClick={() => void repairHosts()}
          >
            {hosts?.ok ? "Repair / re-trust CA" : "Fix hosts"}
          </button>
        </div>
      </div>

      <div className="section-head" style={{ marginTop: 22 }}>
        <h2>Ports</h2>
        <p>
          Run DevStack beside WAMP (Windows) or MAMP PRO (Mac). Non-default
          ports appear in site URLs (<code>https://app.test:8443</code>);{" "}
          <code>.test</code> hosts entries stay the same.
        </p>
      </div>
      <div className="panel-card settings-list">
        <div className="settings-row">
          <label>
            Coexistence mode
            <span className="hint">
              Web 8080/8443 · MySQL 3307 · MariaDB 3308 · Postgres 5433 · Mongo
              27018
            </span>
          </label>
          <button
            type="button"
            className={`toggle ${ports?.coexistenceMode ? "on" : ""}`}
            disabled={busy || !ports}
            onClick={() => void toggleCoexistence()}
          />
        </div>
        {PORT_FIELDS.map((field) => (
          <div className="settings-row" key={field.key}>
            <label>
              {field.label}
              <span className="hint">{field.hint}</span>
            </label>
            <input
              type="number"
              min={1}
              max={65535}
              style={{ width: 100 }}
              disabled={!ports || busy}
              value={ports ? Number(ports[field.key]) : ""}
              onChange={(e) => {
                if (!ports) return;
                const n = Number(e.target.value);
                if (!Number.isFinite(n)) return;
                setPorts({ ...ports, [field.key]: n });
              }}
              onBlur={() => {
                if (ports) void savePorts(ports);
              }}
            />
          </div>
        ))}
        <div className="settings-row">
          <label>
            phpMyAdmin URL port
            <span className="hint">Uses Nginx HTTPS (served by Nginx)</span>
          </label>
          <span className="muted">{ports?.phpmyadmin ?? "…"}</span>
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

      <ConfigTransferSection />

      <div className="section-head" style={{ marginTop: 28 }}>
        <h2>Developer</h2>
        <p>Diagnostics for development builds — turn redaction on before release.</p>
      </div>
      <div className="settings-card" data-testid="developer-settings">
        <div className="settings-row">
          <label>
            Show Logs tab
            <span className="hint">Sidebar Logs stream (default on)</span>
          </label>
          <input
            type="checkbox"
            checked={developer.showLogsTab}
            disabled={busy}
            onChange={(e) =>
              void saveDeveloper({ showLogsTab: e.target.checked })
            }
            data-testid="dev-show-logs"
          />
        </div>
        <div className="settings-row">
          <label>
            Redact secrets
            <span className="hint">
              Off = full command lines, args, env, stdout/stderr (dev default)
            </span>
          </label>
          <input
            type="checkbox"
            checked={developer.redactSecrets}
            disabled={busy}
            onChange={(e) =>
              void saveDeveloper({ redactSecrets: e.target.checked })
            }
            data-testid="dev-redact-secrets"
          />
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
    <div className="placeholder-page">
      <div className="section-head">
        <h2>{title}</h2>
        <p>{blurb}</p>
      </div>
    </div>
  );
}
