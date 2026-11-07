import { useEffect, useState } from "react";
import * as client from "../client";
import {
  browseDatabaseDump,
  openInBrowser,
  openInEditor,
  openInFileManager,
} from "../open";
import type {
  CreateWordPressResult,
  InstalledPackage,
  JobProgress,
  SiteDetails,
  StackStatus,
  TemplateRecord,
} from "../types";

export function SitesPage({ initialSiteId }: { initialSiteId?: string | null }) {
  const [details, setDetails] = useState<SiteDetails[]>([]);
  const [templates, setTemplates] = useState<TemplateRecord[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(
    initialSiteId ?? null,
  );
  const [phpInstalled, setPhpInstalled] = useState<InstalledPackage[]>([]);
  const [nodeInstalled, setNodeInstalled] = useState<InstalledPackage[]>([]);
  const [stack, setStack] = useState<StackStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [showNewWp, setShowNewWp] = useState(false);
  const [showDuplicate, setShowDuplicate] = useState(false);
  const [credentials, setCredentials] =
    useState<CreateWordPressResult | null>(null);
  const [lastBackup, setLastBackup] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const [d, php, node, st, tpl] = await Promise.all([
        client.listSiteDetails(),
        client.listInstalled("php"),
        client.listInstalled("node"),
        client.getStackStatus(),
        client.getTemplates(),
      ]);
      setDetails(d);
      setPhpInstalled(php);
      setNodeInstalled(node);
      setStack(st);
      setTemplates(tpl);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    void refresh();
    let unlisten: (() => void) | undefined;
    void client.onJobProgress((ev) => setProgress(ev)).then((u) => {
      unlisten = u;
    });
    // Re-fetch installed runtimes when returning from PHP/JS install pages.
    const onFocus = () => {
      void refresh();
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onFocus);
    return () => {
      unlisten?.();
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onFocus);
    };
  }, []);

  useEffect(() => {
    if (initialSiteId) setSelectedId(initialSiteId);
  }, [initialSiteId]);

  const selected = details.find((d) => d.site.id === selectedId) ?? null;

  const openSite = (id: string) => {
    setSelectedId(id);
    window.location.hash = `#/sites/${id}`;
  };

  const backToGrid = () => {
    setSelectedId(null);
    window.location.hash = `#/sites`;
  };

  const patch = async (
    siteId: string,
    update: {
      phpVersion?: string | null;
      nodeVersion?: string | null;
      https?: boolean;
    },
  ) => {
    await client.updateSiteEnvironment(siteId, update);
    await refresh();
  };

  const toggleStack = async () => {
    setBusy(true);
    setError(null);
    try {
      if (stack?.nginx) {
        setStack(await client.stopDevStack());
      } else {
        setStack(await client.startDevStack());
      }
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const runJob = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setProgress({
      jobId: "job",
      phase: "start",
      message: "Starting…",
      bytes: 0,
      total: null,
      percent: null,
    });
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  };

  if (selected) {
    return (
      <>
        <SiteDetail
          details={selected}
          phpInstalled={phpInstalled}
          nodeInstalled={nodeInstalled}
          busy={busy}
          lastBackup={lastBackup}
          onBack={backToGrid}
          onPatch={(u) => void patch(selected.site.id, u)}
          onMarkTemplate={() =>
            void runJob(async () => {
              await client.markTemplate(selected.site.id);
            })
          }
          onDuplicate={() => setShowDuplicate(true)}
          onBackup={() =>
            void runJob(async () => {
              const r = await client.backupSiteDatabase(selected.site.id);
              setLastBackup(`${r.path} (${formatBytes(r.bytes)})`);
            })
          }
          onRestore={(path) =>
            void runJob(async () => {
              await client.restoreSiteDatabase(selected.site.id, path);
            })
          }
          onToast={setError}
        />
        {showDuplicate ? (
          <DuplicateModal
            sources={[
              ...templates.map((t) => ({
                id: t.id,
                label: `Template · ${t.name} (${t.sourceDomain})`,
              })),
              ...details.map((d) => ({
                id: d.site.id,
                label: `Site · ${d.site.name} (${d.site.domain})`,
              })),
            ]}
            defaultSource={selected.site.id}
            phpInstalled={phpInstalled}
            busy={busy}
            onClose={() => setShowDuplicate(false)}
            onSubmit={(req) =>
              void runJob(async () => {
                const r = await client.duplicateSite(req);
                setShowDuplicate(false);
                openSite(r.site.id);
              })
            }
          />
        ) : null}
        {progress ? <ProgressToast progress={progress} /> : null}
        {error ? <div className="error-banner">{error}</div> : null}
      </>
    );
  }

  return (
    <div>
      <div className="sites-toolbar">
        <div className="section-head" style={{ margin: 0 }}>
          <h2>Sites</h2>
          <p>Local projects on .test domains — click a tile for details.</p>
        </div>
        <div className="sites-toolbar-actions">
          <span className="muted" style={{ fontSize: 12 }}>
            {stack?.nginx ? "Stack running" : "Stack stopped"}
            {stack?.databaseEngine ? ` · ${stack.databaseEngine}` : ""}
          </span>
          <button
            type="button"
            className="btn btn-install"
            disabled={busy}
            onClick={() => setShowDuplicate(true)}
          >
            Duplicate
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy}
            onClick={() => setShowNewWp(true)}
          >
            New WordPress
          </button>
          <button
            type="button"
            className="btn btn-install"
            disabled={busy}
            onClick={() => void toggleStack()}
          >
            {stack?.nginx ? "Stop stack" : "Start stack"}
          </button>
        </div>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
      {progress ? <ProgressToast progress={progress} /> : null}
      <div className="sites-grid">
        {details.map((d) => (
          <button
            key={d.site.id}
            type="button"
            className="site-tile"
            onClick={() => openSite(d.site.id)}
          >
            <div className="site-tile-top">
              <span
                className={`fw-icon fw-${d.framework}`}
                title={d.framework}
              >
                {d.frameworkIcon}
              </span>
              <span
                className={`status-dot ${
                  d.site.status === "running" ? "on" : ""
                }`}
              />
            </div>
            <div className="site-tile-name">
              {d.site.name}
              {d.site.isTemplate ? (
                <span className="tpl-badge">template</span>
              ) : null}
            </div>
            <div className="site-tile-domain">{d.site.domain}</div>
            <div className="site-tile-meta">
              <span>PHP {d.site.phpVersion ?? "—"}</span>
              <span className="muted">{d.framework}</span>
            </div>
          </button>
        ))}
      </div>

      {showNewWp ? (
        <NewWordpressModal
          phpInstalled={phpInstalled}
          busy={busy}
          onClose={() => setShowNewWp(false)}
          onSubmit={(req) =>
            void runJob(async () => {
              const result = await client.createWordpress(req);
              setCredentials(result);
              setShowNewWp(false);
              openSite(result.site.id);
            })
          }
        />
      ) : null}

      {showDuplicate ? (
        <DuplicateModal
          sources={[
            ...templates.map((t) => ({
              id: t.id,
              label: `Template · ${t.name} (${t.sourceDomain})`,
            })),
            ...details.map((d) => ({
              id: d.site.id,
              label: `Site · ${d.site.name} (${d.site.domain})`,
            })),
          ]}
          phpInstalled={phpInstalled}
          busy={busy}
          onClose={() => setShowDuplicate(false)}
          onSubmit={(req) =>
            void runJob(async () => {
              const r = await client.duplicateSite(req);
              setShowDuplicate(false);
              openSite(r.site.id);
            })
          }
        />
      ) : null}

      {credentials ? (
        <CredentialsModal
          result={credentials}
          onClose={() => setCredentials(null)}
        />
      ) : null}
    </div>
  );
}

function ProgressToast({ progress }: { progress: JobProgress }) {
  const pct =
    progress.percent ??
    (progress.total && progress.total > 0
      ? Math.min(100, (progress.bytes / progress.total) * 100)
      : null);
  return (
    <div className="job-progress" role="status">
      <div className="job-progress-head">
        <strong>{progress.phase}</strong>
        <span className="muted">{progress.message}</span>
      </div>
      <div className="job-progress-bar">
        <div
          className="job-progress-fill"
          style={{ width: pct != null ? `${pct}%` : "40%" }}
        />
      </div>
    </div>
  );
}

function runtimeValue(pkg: InstalledPackage): string {
  return pkg.channel || pkg.version;
}

function NewWordpressModal({
  phpInstalled,
  busy,
  onClose,
  onSubmit,
}: {
  phpInstalled: InstalledPackage[];
  busy: boolean;
  onClose: () => void;
  onSubmit: (req: {
    name: string;
    domain?: string;
    path?: string;
    phpVersion?: string;
    databaseEngine?: string;
    locale?: string;
  }) => void;
}) {
  const [name, setName] = useState("my-site");
  const [domain, setDomain] = useState("my-site.test");
  const [path, setPath] = useState("~/Sites/my-site");
  const [php, setPhp] = useState(
    phpInstalled[0] ? runtimeValue(phpInstalled[0]) : "",
  );
  const [engine, setEngine] = useState("mariadb");
  const [locale, setLocale] = useState("en_US");

  useEffect(() => {
    const slug = name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "");
    setDomain(`${slug || "site"}.test`);
    setPath(`~/Sites/${slug || "site"}`);
  }, [name]);

  return (
    <div className="modal-backdrop" role="dialog">
      <div className="modal-card">
        <h3>New WordPress</h3>
        <p className="muted">
          Downloads latest WordPress + WP-CLI, creates a database, installs over
          HTTPS on a .test domain.
        </p>
        <label className="field">
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="field">
          Domain
          <input value={domain} onChange={(e) => setDomain(e.target.value)} />
        </label>
        <label className="field">
          Directory
          <input value={path} onChange={(e) => setPath(e.target.value)} />
        </label>
        <div className="field-row">
          <label className="field">
            PHP
            {phpInstalled.length === 0 ? (
              <span className="hint">
                No PHP installed —{" "}
                <a href="#/php" onClick={onClose}>
                  install one
                </a>
              </span>
            ) : (
              <select
                className="native"
                value={php}
                onChange={(e) => setPhp(e.target.value)}
              >
                {phpInstalled.map((v) => (
                  <option key={v.version} value={runtimeValue(v)}>
                    {runtimeValue(v)}
                  </option>
                ))}
              </select>
            )}
          </label>
          <label className="field">
            Database
            <select
              className="native"
              value={engine}
              onChange={(e) => setEngine(e.target.value)}
            >
              <option value="mariadb">MariaDB</option>
              <option value="mysql">MySQL</option>
            </select>
          </label>
          <label className="field">
            Locale
            <select
              className="native"
              value={locale}
              onChange={(e) => setLocale(e.target.value)}
            >
              <option value="en_US">en_US</option>
              <option value="pl_PL">pl_PL</option>
            </select>
          </label>
        </div>
        <div className="modal-actions">
          <button type="button" className="btn btn-install" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || !name.trim() || !php}
            onClick={() =>
              onSubmit({
                name: name.trim(),
                domain,
                path,
                phpVersion: php,
                databaseEngine: engine,
                locale,
              })
            }
          >
            Create site
          </button>
        </div>
      </div>
    </div>
  );
}

function DuplicateModal({
  sources,
  defaultSource,
  phpInstalled,
  busy,
  onClose,
  onSubmit,
}: {
  sources: { id: string; label: string }[];
  defaultSource?: string;
  phpInstalled: InstalledPackage[];
  busy: boolean;
  onClose: () => void;
  onSubmit: (req: {
    sourceId: string;
    name: string;
    domain?: string;
    path: string;
    phpVersion?: string;
    databaseEngine?: string;
  }) => void;
}) {
  const [sourceId, setSourceId] = useState(
    defaultSource || sources[0]?.id || "",
  );
  const [name, setName] = useState("copy-site");
  const [domain, setDomain] = useState("copy-site.test");
  const [path, setPath] = useState("~/Sites/copy-site");
  const [php, setPhp] = useState(
    phpInstalled[0] ? runtimeValue(phpInstalled[0]) : "",
  );
  const [engine, setEngine] = useState("mariadb");

  useEffect(() => {
    const slug = name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "");
    setDomain(`${slug || "site"}.test`);
    setPath(`~/Sites/${slug || "site"}`);
  }, [name]);

  return (
    <div className="modal-backdrop" role="dialog">
      <div className="modal-card">
        <h3>Duplicate site</h3>
        <p className="muted">
          Copies files, streams the database import, then runs serialized-safe{" "}
          <code>wp search-replace</code>.
        </p>
        <label className="field">
          Source template or site
          <select
            className="native"
            value={sourceId}
            onChange={(e) => setSourceId(e.target.value)}
          >
            {sources.map((s) => (
              <option key={s.id} value={s.id}>
                {s.label}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          New name
          <input value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="field">
          New domain
          <input value={domain} onChange={(e) => setDomain(e.target.value)} />
        </label>
        <label className="field">
          Target directory
          <input value={path} onChange={(e) => setPath(e.target.value)} />
        </label>
        <div className="field-row">
          <label className="field">
            PHP
            {phpInstalled.length === 0 ? (
              <span className="hint">
                No PHP installed —{" "}
                <a href="#/php" onClick={onClose}>
                  install one
                </a>
              </span>
            ) : (
              <select
                className="native"
                value={php}
                onChange={(e) => setPhp(e.target.value)}
              >
                {phpInstalled.map((v) => (
                  <option key={v.version} value={runtimeValue(v)}>
                    {runtimeValue(v)}
                  </option>
                ))}
              </select>
            )}
          </label>
          <label className="field">
            Database
            <select
              className="native"
              value={engine}
              onChange={(e) => setEngine(e.target.value)}
            >
              <option value="mariadb">MariaDB</option>
              <option value="mysql">MySQL</option>
            </select>
          </label>
        </div>
        <div className="modal-actions">
          <button type="button" className="btn btn-install" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={
              busy || !sourceId || !name.trim() || !path.trim() || !php
            }
            onClick={() =>
              onSubmit({
                sourceId,
                name: name.trim(),
                domain,
                path,
                phpVersion: php,
                databaseEngine: engine,
              })
            }
          >
            Duplicate
          </button>
        </div>
      </div>
    </div>
  );
}

function CredentialsModal({
  result,
  onClose,
}: {
  result: CreateWordPressResult;
  onClose: () => void;
}) {
  return (
    <div className="modal-backdrop" role="dialog">
      <div className="modal-card">
        <h3>WordPress ready</h3>
        <p>
          Admin credentials are shown <strong>once</strong> — copy them now.
        </p>
        <div className="cred-box">
          <div>
            <span className="muted">URL</span>
            <div>{result.url}</div>
          </div>
          <div>
            <span className="muted">User</span>
            <div>
              <code>{result.adminUser}</code>
            </div>
          </div>
          <div>
            <span className="muted">Password</span>
            <div>
              <code>{result.adminPassword}</code>
            </div>
          </div>
          <div>
            <span className="muted">Email</span>
            <div>{result.adminEmail}</div>
          </div>
        </div>
        <div className="modal-actions">
          <button
            type="button"
            className="btn btn-install"
            onClick={() => {
              void openInBrowser(result.url).catch((e) =>
                console.error(e instanceof Error ? e.message : String(e)),
              );
            }}
          >
            Open in browser
          </button>
          <button type="button" className="btn btn-primary" onClick={onClose}>
            Done
          </button>
        </div>
      </div>
    </div>
  );
}

function SiteDetail({
  details,
  phpInstalled,
  nodeInstalled,
  busy,
  lastBackup,
  onBack,
  onPatch,
  onMarkTemplate,
  onDuplicate,
  onBackup,
  onRestore,
  onToast,
}: {
  details: SiteDetails;
  phpInstalled: InstalledPackage[];
  nodeInstalled: InstalledPackage[];
  busy: boolean;
  lastBackup: string | null;
  onBack: () => void;
  onPatch: (u: {
    phpVersion?: string | null;
    nodeVersion?: string | null;
    https?: boolean;
  }) => void;
  onMarkTemplate: () => void;
  onDuplicate: () => void;
  onBackup: () => void;
  onRestore: (path: string) => void;
  onToast: (message: string | null) => void;
}) {
  const site = details.site;
  const [restorePath, setRestorePath] = useState("");
  const [backupsDir, setBackupsDir] = useState<string | null>(null);

  useEffect(() => {
    void client
      .getBackupsDir()
      .then(setBackupsDir)
      .catch((e) =>
        onToast(e instanceof Error ? e.message : String(e)),
      );
  }, [onToast]);

  const runOpen = (fn: () => Promise<void>) => {
    void fn().catch((e) =>
      onToast(e instanceof Error ? e.message : String(e)),
    );
  };

  const phpOptions = phpInstalled.map((p) => runtimeValue(p));
  const nodeOptions = nodeInstalled.map((p) => runtimeValue(p));
  // Keep current selection visible even if no longer installed.
  if (site.phpVersion && !phpOptions.includes(site.phpVersion)) {
    phpOptions.unshift(site.phpVersion);
  }
  if (site.nodeVersion && !nodeOptions.includes(site.nodeVersion)) {
    nodeOptions.unshift(site.nodeVersion);
  }

  return (
    <div className="site-detail">
      <div className="site-detail-bar">
        <button type="button" className="btn btn-install" onClick={onBack}>
          ← Back
        </button>
        <div className="site-detail-title">
          <span className={`fw-icon fw-${details.framework}`}>
            {details.frameworkIcon}
          </span>
          <div>
            <strong>
              {site.name}
              {site.isTemplate ? (
                <span className="tpl-badge">template</span>
              ) : null}
            </strong>
            <div className="muted" style={{ fontSize: 12 }}>
              {site.domain}
            </div>
          </div>
        </div>
        <div className="row-actions">
          <button
            type="button"
            className="btn btn-primary"
            onClick={() =>
              runOpen(() =>
                openInBrowser(site.https ? details.httpsUrl : details.url),
              )
            }
          >
            Open in browser
          </button>
        </div>
      </div>

      <div className="site-detail-grid">
        <div className="panel-card settings-list">
          <div className="settings-row" style={{ background: "#fafafa" }}>
            <label>Environment</label>
          </div>
          <div className="settings-row">
            <label>
              PHP
              {phpInstalled.length === 0 ? (
                <span className="hint">No versions installed yet</span>
              ) : null}
            </label>
            {phpInstalled.length === 0 ? (
              <button
                type="button"
                className="btn btn-install"
                onClick={() => {
                  window.location.hash = "#/php";
                }}
              >
                Install PHP
              </button>
            ) : (
              <select
                className="native"
                value={site.phpVersion ?? ""}
                onChange={(e) =>
                  onPatch({ phpVersion: e.target.value || null })
                }
              >
                <option value="">System</option>
                {phpOptions.map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </select>
            )}
          </div>
          <div className="settings-row">
            <label>
              Node
              {nodeInstalled.length === 0 ? (
                <span className="hint">No versions installed yet</span>
              ) : null}
            </label>
            {nodeInstalled.length === 0 ? (
              <button
                type="button"
                className="btn btn-install"
                onClick={() => {
                  window.location.hash = "#/javascript";
                }}
              >
                Install Node
              </button>
            ) : (
              <select
                className="native"
                value={site.nodeVersion ?? ""}
                onChange={(e) =>
                  onPatch({ nodeVersion: e.target.value || null })
                }
              >
                <option value="">System</option>
                {nodeOptions.map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </select>
            )}
          </div>
          <div className="settings-row">
            <label>HTTPS</label>
            <button
              type="button"
              className={`toggle ${site.https ? "on" : ""}`}
              onClick={() => onPatch({ https: !site.https })}
            />
          </div>
        </div>

        <div className="panel-card settings-list">
          <div className="settings-row" style={{ background: "#fafafa" }}>
            <label>Paths &amp; database</label>
          </div>
          <div className="settings-row">
            <label>
              Document root
              <span className="hint">{details.absolutePath}</span>
            </label>
            <button
              type="button"
              className="btn btn-install"
              onClick={() =>
                runOpen(() => openInFileManager(details.absolutePath))
              }
            >
              Open folder
            </button>
          </div>
          <div className="settings-row">
            <label>
              Editor
              <span className="hint">Open path with the default app</span>
            </label>
            <button
              type="button"
              className="btn btn-install"
              onClick={() => runOpen(() => openInEditor(details.absolutePath))}
            >
              Open in editor
            </button>
          </div>
          <div className="settings-row">
            <label>
              Database
              <span className="hint">
                {details.databaseName
                  ? `${details.databaseName}${
                      details.databaseUser ? ` · ${details.databaseUser}` : ""
                    }`
                  : "Not detected"}
              </span>
            </label>
            {details.phpmyadminUrl ? (
              <button
                type="button"
                className="btn btn-primary"
                onClick={() =>
                  runOpen(() => openInBrowser(details.phpmyadminUrl!))
                }
              >
                Open database
              </button>
            ) : (
              <span className="muted" style={{ fontSize: 12 }}>
                {details.framework === "wordpress"
                  ? "Add DB_NAME to wp-config.php"
                  : "—"}
              </span>
            )}
          </div>
        </div>
      </div>

      <div className="panel-card settings-list" style={{ marginTop: 16 }}>
        <div className="settings-row" style={{ background: "#fafafa" }}>
          <label>Templates &amp; backups</label>
        </div>
        <div className="settings-row">
          <label>
            Starter template
            <span className="hint">
              Snapshot files + streaming DB dump (no size limit)
            </span>
          </label>
          <div className="row-actions">
            <button
              type="button"
              className="btn btn-install"
              disabled={busy}
              onClick={onMarkTemplate}
            >
              Mark as template
            </button>
            <button
              type="button"
              className="btn btn-install"
              disabled={busy}
              onClick={onDuplicate}
            >
              Duplicate…
            </button>
          </div>
        </div>
        <div className="settings-row">
          <label>
            Database backup
            <span className="hint">
              {lastBackup
                ? `Last: ${lastBackup}`
                : "mysqldump piped through gzip/zstd"}
            </span>
          </label>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || !details.databaseName}
            onClick={onBackup}
          >
            Backup database
          </button>
        </div>
        <div className="settings-row">
          <label>
            Restore
            <span className="hint">
              .sql / .sql.gz / .sql.zst
              {backupsDir ? ` · ${backupsDir}` : ""}
            </span>
          </label>
          <div className="restore-row">
            <input
              value={restorePath}
              placeholder="/path/to/backup.sql.gz"
              onChange={(e) => setRestorePath(e.target.value)}
            />
            <button
              type="button"
              className="btn btn-install"
              disabled={busy || !backupsDir}
              title="Open backups folder"
              onClick={() => {
                if (!backupsDir) return;
                runOpen(() => openInFileManager(backupsDir));
              }}
            >
              Open folder
            </button>
            <button
              type="button"
              className="btn btn-install"
              disabled={busy}
              onClick={() => {
                void browseDatabaseDump({
                  defaultPath: backupsDir ?? undefined,
                })
                  .then((picked) => {
                    if (picked) setRestorePath(picked);
                  })
                  .catch((e) =>
                    onToast(e instanceof Error ? e.message : String(e)),
                  );
              }}
            >
              Browse
            </button>
            <button
              type="button"
              className="btn btn-install"
              disabled={busy || !restorePath.trim() || !details.databaseName}
              onClick={() => onRestore(restorePath.trim())}
            >
              Restore
            </button>
          </div>
        </div>
      </div>

      <div className="danger-zone" style={{ marginTop: 16 }}>
        <button type="button" className="danger-btn" disabled>
          Delete Application
        </button>
      </div>
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
