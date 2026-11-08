import { useEffect, useState } from "react";
import * as client from "../client";
import {
  AsyncModal,
  BusyLabel,
  useAsyncJob,
} from "../components/AsyncModal";
import { PortConflictDialog } from "../components/PortConflictDialog";
import {
  browseDatabaseDump,
  browseFolder,
  openInBrowser,
  openInEditor,
  openInFileManager,
} from "../open";
import { humanizeError, parsePortBusy } from "../portBusy";
import type {
  CreateWordPressResult,
  HostsStatus,
  InstalledPackage,
  JobProgress,
  PortCheckResult,
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
  const [hosts, setHosts] = useState<HostsStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [showNewWp, setShowNewWp] = useState(false);
  const [showAddSite, setShowAddSite] = useState(false);
  const [showDuplicate, setShowDuplicate] = useState(false);
  const [portConflict, setPortConflict] = useState<PortCheckResult | null>(
    null,
  );
  const [credentials, setCredentials] =
    useState<CreateWordPressResult | null>(null);
  const [lastBackup, setLastBackup] = useState<string | null>(null);
  const [patching, setPatching] = useState(false);

  const refresh = async () => {
    try {
      const [d, php, node, st, tpl, h] = await Promise.all([
        client.listSiteDetails(),
        client.listInstalled("php"),
        client.listInstalled("node"),
        client.getStackStatus(),
        client.getTemplates(),
        client.getHostsStatus(),
      ]);
      setDetails(d);
      setPhpInstalled(php);
      setNodeInstalled(node);
      setStack(st);
      setTemplates(tpl);
      setHosts(h);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const repairHosts = async () => {
    setBusy(true);
    setError(null);
    try {
      const h = await client.repairHosts();
      setHosts(h);
      if (!h.ok) {
        setError(h.message ?? "Hosts repair incomplete");
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const openWithHostsCheck = async (url: string) => {
    const h = await client.getHostsStatus();
    setHosts(h);
    if (!h.ok) {
      setError(
        h.message ??
          "Hosts file is missing DevStack entries. Click Fix hosts, then try again.",
      );
      return;
    }
    await openInBrowser(url);
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
      webServer?: string | null;
    },
  ) => {
    setError(null);
    setPatching(true);
    // Optimistic UI — reflect the toggle/select immediately while backend reloads.
    setDetails((prev) =>
      prev.map((d) => {
        if (d.site.id !== siteId) return d;
        const site = { ...d.site };
        if (update.phpVersion !== undefined) site.phpVersion = update.phpVersion;
        if (update.nodeVersion !== undefined)
          site.nodeVersion = update.nodeVersion;
        if (update.https !== undefined) site.https = update.https;
        if (update.webServer !== undefined) site.webServer = update.webServer;
        return { ...d, site };
      }),
    );
    try {
      await client.updateSiteEnvironment(siteId, update);
      await refresh();
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
      await refresh();
    } finally {
      setPatching(false);
    }
  };

  const stackRunning = Boolean(stack?.nginx || stack?.apache);

  const toggleStack = async () => {
    setBusy(true);
    setError(null);
    setPortConflict(null);
    try {
      if (stackRunning) {
        setStack(await client.stopDevStack());
      } else {
        // Preflight nginx ports so the dialog can offer suggested/coexistence.
        const check = await client.checkServicePorts("nginx");
        if (!check.ok) {
          setPortConflict(check);
          setError(check.message ?? "Ports are in use");
          return;
        }
        setStack(await client.startDevStack());
      }
      await refresh();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const busyPorts = parsePortBusy(msg);
      if (busyPorts) {
        setPortConflict(busyPorts);
        setError(busyPorts.message ?? humanizeError(msg));
      } else {
        setError(humanizeError(msg));
      }
    } finally {
      setBusy(false);
    }
  };

  const resolvePortConflict = async (mode: "suggested" | "coexistence") => {
    if (!portConflict) return;
    setBusy(true);
    setError(null);
    try {
      if (mode === "coexistence") {
        await client.setCoexistenceMode(true);
      } else if (portConflict.serviceId) {
        await client.applyNextFreePorts(portConflict.serviceId);
      }
      setPortConflict(null);
      setStack(await client.startDevStack());
      await refresh();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const busyPorts = parsePortBusy(msg);
      if (busyPorts) {
        setPortConflict(busyPorts);
        setError(busyPorts.message ?? humanizeError(msg));
      } else {
        setError(humanizeError(msg));
      }
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
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
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
          busy={busy || patching}
          patching={patching}
          lastBackup={lastBackup}
          hostsOk={hosts?.ok !== false}
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
          onOpenInBrowser={(url) => void openWithHostsCheck(url)}
          onFixHosts={() => void repairHosts()}
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
            onClose={() => setShowDuplicate(false)}
            onSubmit={async (req) => {
              const r = await client.duplicateSite(req);
              setShowDuplicate(false);
              await refresh();
              openSite(r.site.id);
            }}
          />
        ) : null}
        {progress && !showDuplicate ? (
          <ProgressToast progress={progress} />
        ) : null}
        {hosts && !hosts.ok ? (
          <div
            className="error-banner"
            style={{ display: "flex", gap: 12, alignItems: "center" }}
          >
            <span style={{ flex: 1 }}>
              {hosts.message ??
                "Hosts file is missing DevStack entries for .test domains."}
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
        {error ? <div className="error-banner">{error}</div> : null}
        {credentials ? (
          <CredentialsModal
            result={credentials}
            onOpenBrowser={(url) => void openWithHostsCheck(url)}
            onClose={() => {
              const id = credentials.site.id;
              setCredentials(null);
              openSite(id);
            }}
          />
        ) : null}
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
            {stackRunning ? "Stack running" : "Stack stopped"}
            {stack?.nginx ? " · Nginx" : ""}
            {stack?.apache ? " · Apache" : ""}
            {stack?.databaseEngine ? ` · ${stack.databaseEngine}` : ""}
          </span>
          <button
            type="button"
            className="btn btn-install"
            disabled={busy}
            title="Create an empty site or link an existing folder"
            onClick={() => setShowAddSite(true)}
          >
            Add site
          </button>
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
            title={stackRunning ? "Stop Nginx/Apache/database" : "Start the local stack"}
            onClick={() => void toggleStack()}
          >
            {stackRunning ? "Stop stack" : "Start stack"}
          </button>
        </div>
      </div>
      {hosts && !hosts.ok ? (
        <div
          className="error-banner"
          style={{ display: "flex", gap: 12, alignItems: "center" }}
        >
          <span style={{ flex: 1 }}>
            {hosts.message ??
              "Hosts file is missing DevStack entries for .test domains."}
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
      {stack?.portNotice && stack.portNotice.length > 0 ? (
        <div className="port-notice-banner" role="status">
          {stack.portNotice.map((line) => (
            <p key={line}>{line}</p>
          ))}
        </div>
      ) : null}
      {error ? <div className="error-banner">{error}</div> : null}
      {progress ? <ProgressToast progress={progress} /> : null}
      {details.length === 0 ? (
        <div className="empty-state" style={{ padding: "48px 24px", textAlign: "center" }}>
          <h3 style={{ marginTop: 0 }}>No sites yet</h3>
          <p className="muted">
            Add an empty PHP site, link an existing folder, or create WordPress
            in one click.
          </p>
          <div style={{ display: "flex", gap: 8, justifyContent: "center", flexWrap: "wrap" }}>
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy}
              onClick={() => setShowAddSite(true)}
            >
              Add site
            </button>
            <button
              type="button"
              className="btn btn-install"
              disabled={busy}
              onClick={() => setShowNewWp(true)}
            >
              New WordPress
            </button>
          </div>
        </div>
      ) : (
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
                    (d.reachability?.state ?? d.site.status) === "running"
                      ? "on"
                      : (d.reachability?.state ?? d.site.status) === "error"
                        ? "error"
                        : (d.reachability?.state ?? d.site.status) === "warning"
                          ? "warn"
                          : ""
                  }`}
                  title={
                    d.reachability?.message ??
                    d.reachability?.state ??
                    d.site.status ??
                    "stopped"
                  }
                />
              </div>
              <div className="site-tile-name">
                {d.site.name}
                {d.site.isTemplate ? (
                  <span className="tpl-badge">template</span>
                ) : null}
              </div>
              <div className="site-tile-domain">{d.site.domain}</div>
              {d.reachability?.message ? (
                <div className="site-tile-meta" style={{ color: "var(--danger, #ff3b30)" }}>
                  {d.reachability.message}
                </div>
              ) : null}
              <div className="site-tile-meta">
                <span>PHP {d.site.phpVersion ?? "—"}</span>
                <span className="muted">
                  {(d.site.webServer ?? "nginx").toUpperCase()}
                </span>
                <span className="muted">{d.framework}</span>
              </div>
            </button>
          ))}
        </div>
      )}

      {portConflict && !portConflict.ok ? (
        <PortConflictDialog
          conflict={portConflict}
          busy={busy}
          error={error}
          busyLabel="Updating ports…"
          onCancel={() => {
            setPortConflict(null);
            setError(null);
          }}
          onUseSuggested={() => void resolvePortConflict("suggested")}
          onCoexistence={() => void resolvePortConflict("coexistence")}
        />
      ) : null}

      {showAddSite ? (
        <AddSiteModal
          phpInstalled={phpInstalled}
          onClose={() => setShowAddSite(false)}
          onSubmit={async (req) => {
            const site = await client.createSite(
              req.name,
              req.path,
              req.phpVersion,
              {
                webServer: req.webServer,
                https: req.https,
              },
            );
            setShowAddSite(false);
            await refresh();
            openSite(site.id);
          }}
        />
      ) : null}

      {showNewWp ? (
        <NewWordpressModal
          phpInstalled={phpInstalled}
          onClose={() => setShowNewWp(false)}
          onSubmit={async (req) => {
            const result = await client.createWordpress(req);
            setShowNewWp(false);
            await refresh();
            setCredentials(result);
            // Keep credentials modal visible — do not navigate away yet.
          }}
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
          onClose={() => setShowDuplicate(false)}
          onSubmit={async (req) => {
            const r = await client.duplicateSite(req);
            setShowDuplicate(false);
            await refresh();
            openSite(r.site.id);
          }}
        />
      ) : null}

      {credentials ? (
        <CredentialsModal
          result={credentials}
          onOpenBrowser={(url) => void openWithHostsCheck(url)}
          onClose={() => {
            const id = credentials.site.id;
            setCredentials(null);
            openSite(id);
          }}
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

function AddSiteModal({
  phpInstalled,
  onClose,
  onSubmit,
}: {
  phpInstalled: InstalledPackage[];
  onClose: () => void;
  onSubmit: (req: {
    name: string;
    path: string;
    phpVersion?: string;
    webServer: string;
    https: boolean;
  }) => Promise<void>;
}) {
  const job = useAsyncJob();
  const [mode, setMode] = useState<"empty" | "link">("empty");
  const [name, setName] = useState("my-site");
  const [path, setPath] = useState("~/Sites/my-site");
  const [php, setPhp] = useState(
    phpInstalled[0] ? runtimeValue(phpInstalled[0]) : "",
  );
  const [webServer, setWebServer] = useState("nginx");
  const [https, setHttps] = useState(true);
  const [pickError, setPickError] = useState<string | null>(null);

  useEffect(() => {
    if (mode !== "empty") return;
    const slug = name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "");
    setPath(`~/Sites/${slug || "site"}`);
  }, [name, mode]);

  const pickFolder = async () => {
    setPickError(null);
    try {
      const folder = await browseFolder({ title: "Link existing site folder" });
      if (folder) {
        setPath(folder);
        setMode("link");
        if (!name || name === "my-site") {
          const base = folder.replace(/[\\/]+$/, "").split(/[\\/]/).pop();
          if (base) setName(base);
        }
      }
    } catch (e) {
      setPickError(e instanceof Error ? e.message : String(e));
    }
  };

  const submitLabel = mode === "link" ? "Link site" : "Create site";

  return (
    <AsyncModal
      title="Add site"
      description="Create an empty PHP site or link a folder you already have."
      busy={job.busy}
      progress={job.progress}
      error={job.error ?? pickError}
      onClose={() => {
        if (!job.busy) onClose();
      }}
      actions={
        <>
          <button
            type="button"
            className="btn"
            disabled={job.busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={job.busy || !name.trim() || !path.trim()}
            title={
              !name.trim() || !path.trim()
                ? "Name and directory are required"
                : "Create site and sync hosts"
            }
            onClick={() =>
              void job.run(async () => {
                await onSubmit({
                  name: name.trim(),
                  path: path.trim(),
                  phpVersion: php || undefined,
                  webServer,
                  https,
                });
              })
            }
          >
            <BusyLabel
              busy={job.busy}
              idle={submitLabel}
              busyText="Creating…"
              progress={job.progress}
            />
          </button>
        </>
      }
    >
      <div className="field-row" style={{ gap: 8 }}>
        <button
          type="button"
          className={`btn ${mode === "empty" ? "btn-primary" : "btn-install"}`}
          onClick={() => setMode("empty")}
        >
          Empty site
        </button>
        <button
          type="button"
          className={`btn ${mode === "link" ? "btn-primary" : "btn-install"}`}
          onClick={() => void pickFolder()}
        >
          Link folder…
        </button>
      </div>
      <label className="field">
        Name
        <input value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <label className="field">
        Directory
        <div style={{ display: "flex", gap: 8 }}>
          <input
            style={{ flex: 1 }}
            value={path}
            onChange={(e) => setPath(e.target.value)}
          />
          <button
            type="button"
            className="btn btn-install"
            title="Browse for a folder"
            onClick={() => void pickFolder()}
          >
            Browse
          </button>
        </div>
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
          Web server
          <select
            className="native"
            value={webServer}
            onChange={(e) => setWebServer(e.target.value)}
          >
            <option value="nginx">Nginx</option>
            <option value="apache">Apache</option>
          </select>
        </label>
      </div>
      <label className="field" style={{ flexDirection: "row", gap: 8 }}>
        <input
          type="checkbox"
          checked={https}
          onChange={(e) => setHttps(e.target.checked)}
        />
        HTTPS (.test certificate)
      </label>
    </AsyncModal>
  );
}

function NewWordpressModal({
  phpInstalled,
  onClose,
  onSubmit,
}: {
  phpInstalled: InstalledPackage[];
  onClose: () => void;
  onSubmit: (req: {
    name: string;
    domain?: string;
    path?: string;
    phpVersion?: string;
    databaseEngine?: string;
    locale?: string;
  }) => Promise<void>;
}) {
  const job = useAsyncJob();
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
    <AsyncModal
      title="New WordPress"
      description="Downloads latest WordPress + WP-CLI, creates a database, installs over HTTPS on a .test domain."
      busy={job.busy}
      progress={job.progress}
      error={job.error}
      onClose={() => {
        if (!job.busy) onClose();
      }}
      actions={
        <>
          <button
            type="button"
            className="btn btn-install"
            disabled={job.busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={job.busy || !name.trim() || !php}
            onClick={() =>
              void job.run(async () => {
                await onSubmit({
                  name: name.trim(),
                  domain,
                  path,
                  phpVersion: php,
                  databaseEngine: engine,
                  locale,
                });
              })
            }
          >
            <BusyLabel
              busy={job.busy}
              idle="Create site"
              busyText="Installing WordPress…"
              progress={job.progress}
            />
          </button>
        </>
      }
    >
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
    </AsyncModal>
  );
}

function DuplicateModal({
  sources,
  defaultSource,
  phpInstalled,
  onClose,
  onSubmit,
}: {
  sources: { id: string; label: string }[];
  defaultSource?: string;
  phpInstalled: InstalledPackage[];
  onClose: () => void;
  onSubmit: (req: {
    sourceId: string;
    name: string;
    domain?: string;
    path: string;
    phpVersion?: string;
    databaseEngine?: string;
  }) => Promise<void>;
}) {
  const job = useAsyncJob();
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
    <AsyncModal
      title="Duplicate site"
      description={
        <>
          Copies files, streams the database import, then runs serialized-safe{" "}
          <code>wp search-replace</code>.
        </>
      }
      busy={job.busy}
      progress={job.progress}
      error={job.error}
      onClose={() => {
        if (!job.busy) onClose();
      }}
      actions={
        <>
          <button
            type="button"
            className="btn btn-install"
            disabled={job.busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={
              job.busy || !sourceId || !name.trim() || !path.trim() || !php
            }
            onClick={() =>
              void job.run(async () => {
                await onSubmit({
                  sourceId,
                  name: name.trim(),
                  domain,
                  path,
                  phpVersion: php,
                  databaseEngine: engine,
                });
              })
            }
          >
            <BusyLabel
              busy={job.busy}
              idle="Duplicate"
              busyText="Duplicating…"
              progress={job.progress}
            />
          </button>
        </>
      }
    >
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
    </AsyncModal>
  );
}

function CredentialsModal({
  result,
  onClose,
  onOpenBrowser,
}: {
  result: CreateWordPressResult;
  onClose: () => void;
  onOpenBrowser: (url: string) => void;
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
            onClick={() => onOpenBrowser(result.url)}
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
  patching,
  lastBackup,
  hostsOk,
  onBack,
  onPatch,
  onMarkTemplate,
  onDuplicate,
  onBackup,
  onRestore,
  onOpenInBrowser,
  onFixHosts,
  onToast,
}: {
  details: SiteDetails;
  phpInstalled: InstalledPackage[];
  nodeInstalled: InstalledPackage[];
  busy: boolean;
  patching?: boolean;
  lastBackup: string | null;
  hostsOk: boolean;
  onBack: () => void;
  onPatch: (u: {
    phpVersion?: string | null;
    nodeVersion?: string | null;
    https?: boolean;
    webServer?: string | null;
  }) => void;
  onMarkTemplate: () => void;
  onDuplicate: () => void;
  onBackup: () => void;
  onRestore: (path: string) => void;
  onOpenInBrowser: (url: string) => void;
  onFixHosts: () => void;
  onToast: (message: string | null) => void;
}) {
  const site = details.site;
  const [restorePath, setRestorePath] = useState("");
  const [importPath, setImportPath] = useState("");
  const [srFrom, setSrFrom] = useState("");
  const [srTo, setSrTo] = useState("");
  const [backupsDir, setBackupsDir] = useState<string | null>(null);
  const webServer = (site.webServer ?? "nginx").toLowerCase();

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
              {site.domain} · {webServer === "apache" ? "Apache" : "Nginx"}
            </div>
          </div>
        </div>
        <div className="row-actions">
          {!hostsOk ? (
            <button
              type="button"
              className="btn btn-install"
              disabled={busy}
              onClick={onFixHosts}
            >
              Fix hosts
            </button>
          ) : null}
          <button
            type="button"
            className="btn btn-primary"
            onClick={() =>
              onOpenInBrowser(site.https ? details.httpsUrl : details.url)
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
              Web server
              {patching ? (
                <span className="hint btn-busy-label">
                  <span className="btn-spinner" aria-hidden />
                  Applying…
                </span>
              ) : null}
            </label>
            <select
              className="native"
              value={webServer === "apache" ? "apache" : "nginx"}
              disabled={busy}
              onChange={(e) => onPatch({ webServer: e.target.value })}
            >
              <option value="apache">Apache (full .htaccess)</option>
              <option value="nginx">Nginx</option>
            </select>
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
                disabled={busy}
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
                disabled={busy}
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
            <label>
              HTTPS
              {patching ? (
                <span className="hint btn-busy-label">
                  <span className="btn-spinner" aria-hidden />
                  Applying…
                </span>
              ) : null}
            </label>
            <button
              type="button"
              className={`toggle ${site.https ? "on" : ""}`}
              disabled={busy}
              aria-busy={patching || undefined}
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
            {details.phpmyadminUrl || details.databaseName ? (
              <button
                type="button"
                className="btn btn-primary"
                disabled={busy}
                onClick={() =>
                  runOpen(async () => {
                    const url = await client.mintPhpmyadminSignonUrl(site.id);
                    await openInBrowser(url);
                  })
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
            Download database
            <span className="hint">Save a streamed dump to any path</span>
          </label>
          <button
            type="button"
            className="btn btn-install"
            disabled={busy || !details.databaseName}
            onClick={() => {
              const dest = window.prompt(
                "Save dump as (.sql.gz):",
                `~/Downloads/${site.id}.sql.gz`,
              );
              if (!dest?.trim()) return;
              void (async () => {
                try {
                  const r = await client.downloadSiteDatabase(
                    site.id,
                    dest.trim(),
                  );
                  window.alert(`Saved ${formatBytes(r.bytes)} → ${r.path}`);
                } catch (e) {
                  window.alert(e instanceof Error ? e.message : String(e));
                }
              })();
            }}
          >
            Download…
          </button>
        </div>
        <div className="settings-row">
          <label>
            Import database
            <span className="hint">
              .sql / .sql.gz / .zst / .zip — WordPress auto search-replace
            </span>
          </label>
          <div className="restore-row">
            <input
              value={importPath}
              placeholder="/path/to/dump.sql.gz"
              onChange={(e) => setImportPath(e.target.value)}
            />
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy || !importPath.trim() || !details.databaseName}
              onClick={() => {
                void (async () => {
                  try {
                    await client.importSiteDatabaseFile(
                      site.id,
                      importPath.trim(),
                      srFrom.trim() || null,
                      srTo.trim() || null,
                    );
                    window.alert("Import complete (URL rewrite applied when WP)");
                  } catch (e) {
                    window.alert(e instanceof Error ? e.message : String(e));
                  }
                })();
              }}
            >
              Import
            </button>
          </div>
        </div>
        {details.framework !== "wordpress" ? (
          <div className="settings-row">
            <label>
              Optional search → replace
              <span className="hint">Non-WordPress URL rewrite</span>
            </label>
            <div className="restore-row">
              <input
                value={srFrom}
                placeholder="old URL"
                onChange={(e) => setSrFrom(e.target.value)}
              />
              <input
                value={srTo}
                placeholder="new URL"
                onChange={(e) => setSrTo(e.target.value)}
              />
            </div>
          </div>
        ) : null}
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
