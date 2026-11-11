import { useEffect, useRef, useState } from "react";
import * as client from "../client";
import {
  AsyncModal,
  BusyLabel,
  useAsyncJob,
} from "../components/AsyncModal";
import { PortConflictDialog } from "../components/PortConflictDialog";
import { MobilePreviewModal } from "../components/MobilePreviewModal";
import { SiteBackupsPanel } from "../components/SiteBackupsPanel";
import { SiteDomainPanel } from "../components/SiteDomainPanel";
import { SiteLogsPanel } from "../components/SiteLogsPanel";
import { SitePhpIniPanel } from "../components/SitePhpIniPanel";
import {
  TemplatePickerModal,
  TemplateResultModal,
} from "../components/TemplatePickerModal";
import { SiteTile } from "../components/SiteTile";
import { SitesEmptyState } from "../components/SitesEmptyState";
import { SitesStatusBanner } from "../components/SitesStatusBanner";
import { XdebugHints } from "../components/XdebugHints";
import {
  browseDatabaseDump,
  browseFolder,
  openInBrowser,
  openInEditor,
  openInFileManager,
  siteOpenUrl,
} from "../open";
import { humanizeError, parsePortBusy } from "../portBusy";
import { stackSslPorts } from "../siteUrl";
import { useAppData } from "../store/AppData";
import type {
  CreateFromTemplateResult,
  CreateWordPressResult,
  InstalledPackage,
  JobProgress,
  PortCheckResult,
  SiteDetails,
  SiteRecord,
  StackStatus,
} from "../types";

export function SitesPage({
  initialSiteId,
  active = true,
}: {
  initialSiteId?: string | null;
  /** False while the keep-alive pane is hidden. */
  active?: boolean;
}) {
  const {
    sites: sitesCache,
    loadSites,
    ensureSites,
    setSitesDetails,
    setStack,
    patchSitesCache,
  } = useAppData();
  const details = sitesCache.details;
  const templates = sitesCache.templates;
  const phpInstalled = sitesCache.phpInstalled;
  const nodeInstalled = sitesCache.nodeInstalled;
  const stack = sitesCache.stack;
  const hosts = sitesCache.hosts;
  const sitesLoaded = sitesCache.loaded;
  const sitesRefreshing = sitesCache.refreshing || sitesCache.probing;

  const setDetails = setSitesDetails;
  const [selectedId, setSelectedId] = useState<string | null>(
    initialSiteId ?? null,
  );
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [showNewWp, setShowNewWp] = useState(false);
  const [showAddSite, setShowAddSite] = useState(false);
  const [showDuplicate, setShowDuplicate] = useState(false);
  const [showTemplates, setShowTemplates] = useState(false);
  const [templateResult, setTemplateResult] =
    useState<CreateFromTemplateResult | null>(null);
  const [mobileSite, setMobileSite] = useState<{ id: string; name: string } | null>(
    null,
  );
  const [portConflict, setPortConflict] = useState<PortCheckResult | null>(
    null,
  );
  const [credentials, setCredentials] =
    useState<CreateWordPressResult | null>(null);
  const [lastBackup, setLastBackup] = useState<string | null>(null);
  const [patching, setPatching] = useState(false);
  const [starting, setStarting] = useState(false);
  const [startPhases, setStartPhases] = useState<JobProgress[]>([]);
  const startingRef = useRef(false);
  const wasActive = useRef(active);
  /** Sync guard — `busy` state alone can race before the next render. */
  const stackOpInflight = useRef(false);

  const refresh = async () => {
    try {
      await loadSites({ force: true, probe: true });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const repairHosts = async () => {
    setBusy(true);
    setError(null);
    try {
      const h = await client.repairHosts();
      patchSitesCache({ hosts: h });
      if (!h.ok) {
        setError(h.message ?? "Hosts repair incomplete");
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  /** Open an arbitrary URL (credentials modal, install.php, …). */
  const openWithHostsCheck = async (url: string) => {
    const sslPorts = stackSslPorts(stack);
    let h = await client.getHostsStatus();
    patchSitesCache({ hosts: h });
    if (!h.ok) {
      try {
        h = await client.repairHosts();
        patchSitesCache({ hosts: h });
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        return;
      }
      if (!h.ok) {
        setError(
          h.message ??
            "Hosts file is missing DevStack entries. Click Fix hosts, then try again.",
        );
        return;
      }
    }
    // Scheme rewrite only for known SSL ports — never remaps :8088 → :8444.
    await openInBrowser(url, { sslPorts });
  };

  /** Single Open path for tile + detail — always `formatSiteUrl(site, stack)`. */
  const openSiteWithHostsCheck = async (site: SiteRecord) => {
    await openWithHostsCheck(siteOpenUrl(site, stack));
  };

  useEffect(() => {
    // AppData already boots the cache; only (re)load here when nothing real is cached.
    void ensureSites().catch((e) =>
      setError(e instanceof Error ? e.message : String(e)),
    );
    let unlisten: (() => void) | undefined;
    void client
      .onJobProgress((ev) => {
        setProgress(ev);
        if (startingRef.current) {
          setStartPhases((prev) => {
            const i = prev.findIndex((p) => p.phase === ev.phase);
            if (i === -1) return [...prev, ev];
            const next = [...prev];
            next[i] = ev;
            return next;
          });
        }
      })
      .then((u) => {
        unlisten = u;
      });
    return () => {
      unlisten?.();
    };
  }, [ensureSites]);

  // Soft background refresh when returning to Sites — cached tiles stay visible.
  useEffect(() => {
    if (active && !wasActive.current && sitesLoaded) {
      void loadSites({ force: true, probe: true });
    }
    wasActive.current = active;
  }, [active, loadSites, sitesLoaded]);

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
    const current = details.find((d) => d.site.id === siteId)?.site;
    // Backend always assigns php/node (null = System). Always send the full
    // intended values so HTTPS/web-server toggles do not clear PHP.
    const payload = {
      phpVersion:
        update.phpVersion !== undefined
          ? update.phpVersion
          : (current?.phpVersion ?? null),
      nodeVersion:
        update.nodeVersion !== undefined
          ? update.nodeVersion
          : (current?.nodeVersion ?? null),
      https: update.https,
      webServer: update.webServer,
    };
    // Optimistic UI — reflect the toggle/select immediately while backend reloads.
    setDetails(
      details.map((d) => {
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
      await client.updateSiteEnvironment(siteId, payload);
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
    if (stackOpInflight.current) return;
    stackOpInflight.current = true;
    setBusy(true);
    setError(null);
    setPortConflict(null);
    const willStart = !stackRunning;
    if (willStart) {
      startingRef.current = true;
      setStartPhases([]);
      setStarting(true);
    }
    try {
      if (!willStart) {
        setStack(await client.stopDevStack());
      } else {
        // Auto-separates busy ports — do not block on a preflight modal, and
        // never open a browser from Start stack. client.startDevStack coalesces
        // concurrent invokes onto one shared promise.
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
      await refresh();
    } finally {
      stackOpInflight.current = false;
      startingRef.current = false;
      setStarting(false);
      setStartPhases([]);
      setProgress(null);
      setBusy(false);
    }
  };

  const tileTerminal = async (siteId: string, path: string) => {
    try {
      await client.openSiteTerminal(siteId);
    } catch (e) {
      try {
        await openInFileManager(path);
      } catch {
        setError(humanizeError(e instanceof Error ? e.message : String(e)));
      }
    }
  };

  const tileDatabase = async (siteId: string) => {
    try {
      await openInBrowser(await client.mintPhpmyadminSignonUrl(siteId));
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    }
  };

  const toggleXdebug = async (siteId: string, enabled: boolean) => {
    setError(null);
    setDetails(
      details.map((d) =>
        d.site.id === siteId
          ? { ...d, site: { ...d.site, xdebug: enabled } }
          : d,
      ),
    );
    try {
      await client.setSiteXdebug(siteId, enabled);
      await refresh();
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
      await refresh();
    }
  };

  const resolvePortConflict = async (mode: "suggested" | "coexistence") => {
    if (!portConflict || stackOpInflight.current) return;
    stackOpInflight.current = true;
    setBusy(true);
    setError(null);
    startingRef.current = true;
    setStartPhases([]);
    setStarting(true);
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
      stackOpInflight.current = false;
      startingRef.current = false;
      setStarting(false);
      setStartPhases([]);
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
          stack={stack}
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
          onOpenInBrowser={() => void openSiteWithHostsCheck(selected.site)}
          onFixHosts={() => void repairHosts()}
          onToast={setError}
          onXdebug={(on) => void toggleXdebug(selected.site.id, on)}
          onTerminal={() =>
            void tileTerminal(selected.site.id, selected.absolutePath)
          }
          onRefresh={() => void refresh()}
          onMobilePreview={() =>
            setMobileSite({ id: selected.site.id, name: selected.site.name })
          }
          onDeleted={(warnings) => {
            if (warnings.length) {
              setError(warnings.join(" · "));
            }
            backToGrid();
            void refresh();
          }}
        />
        {mobileSite ? (
          <MobilePreviewModal
            siteId={mobileSite.id}
            siteName={mobileSite.name}
            onClose={() => setMobileSite(null)}
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
            defaultSource={selected.site.id}
            phpInstalled={phpInstalled}
            onClose={() => setShowDuplicate(false)}
            onSubmit={async (req) => {
              const r = await client.cloneSite(req);
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
      {stack || starting ? (
        <SitesStatusBanner
          running={stackRunning}
          starting={starting}
          phases={startPhases}
          stack={stack}
          busy={busy || starting || stackOpInflight.current}
          onStart={() => void toggleStack()}
          onStop={() => void toggleStack()}
        />
      ) : null}
      <div className="sites-toolbar">
        <p className="sites-toolbar-blurb muted">
          Local projects on .test domains — click a tile for details.
        </p>
        <div className="sites-toolbar-actions">
          {sitesRefreshing ? (
            <span className="muted" style={{ fontSize: 11 }} data-testid="sites-updating">
              Updating…
            </span>
          ) : null}
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
            title="Clone (duplicate) a site or template into a new site"
            onClick={() => setShowDuplicate(true)}
          >
            Clone
          </button>
          <button
            type="button"
            className="btn btn-install"
            disabled={busy}
            title="Laravel, Symfony, Statamic, Drupal, Joomla, WooCommerce"
            data-testid="new-from-template"
            onClick={() => setShowTemplates(true)}
          >
            From template
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy}
            onClick={() => setShowNewWp(true)}
          >
            New WordPress
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
      {progress && !starting ? <ProgressToast progress={progress} /> : null}
      {!sitesLoaded ? (
        <div className="loading" data-testid="sites-loading">
          Loading sites…
        </div>
      ) : details.length === 0 ? (
        <SitesEmptyState
          busy={busy}
          onAddFolder={() => setShowAddSite(true)}
          onCreateWordPress={() => setShowNewWp(true)}
          onTemplates={() => setShowTemplates(true)}
          onImport={() => {
            if (templates.length > 0) setShowDuplicate(true);
            else window.location.hash = "#/backups";
          }}
        />
      ) : (
        <div className="sites-grid" data-testid="sites-grid">
          {details.map((d) => (
            <SiteTile
              key={d.site.id}
              details={d}
              stack={stack}
              phpOptions={phpInstalled.map((p) => runtimeValue(p))}
              busy={busy || patching}
              onOpenDetail={() => openSite(d.site.id)}
              onOpen={() => void openSiteWithHostsCheck(d.site)}
              onOpenUrl={(url) => void openWithHostsCheck(url)}
              onFolder={() =>
                void openInFileManager(d.absolutePath).catch((e) =>
                  setError(e instanceof Error ? e.message : String(e)),
                )
              }
              onTerminal={() => void tileTerminal(d.site.id, d.absolutePath)}
              onDatabase={() => void tileDatabase(d.site.id)}
              onHttps={(https) => void patch(d.site.id, { https })}
              onPhp={(phpVersion) => void patch(d.site.id, { phpVersion })}
              onXdebug={(on) => void toggleXdebug(d.site.id, on)}
              onMobilePreview={() =>
                setMobileSite({ id: d.site.id, name: d.site.name })
              }
            />
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
            const r = await client.cloneSite(req);
            setShowDuplicate(false);
            await refresh();
            openSite(r.site.id);
          }}
        />
      ) : null}

      {showTemplates ? (
        <TemplatePickerModal
          phpOptions={phpInstalled.map((p) => runtimeValue(p))}
          onClose={() => setShowTemplates(false)}
          onCreated={async (result) => {
            setShowTemplates(false);
            await refresh();
            setTemplateResult(result);
          }}
        />
      ) : null}

      {templateResult ? (
        <TemplateResultModal
          result={templateResult}
          onClose={() => {
            const id = templateResult.site.id;
            setTemplateResult(null);
            openSite(id);
          }}
        />
      ) : null}

      {mobileSite ? (
        <MobilePreviewModal
          siteId={mobileSite.id}
          siteName={mobileSite.name}
          onClose={() => setMobileSite(null)}
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
      title="Clone / duplicate site"
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
              idle="Clone"
              busyText="Cloning…"
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
  stack,
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
  onXdebug,
  onTerminal,
  onRefresh,
  onMobilePreview,
  onDeleted,
}: {
  details: SiteDetails;
  stack: StackStatus | null;
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
  onOpenInBrowser: () => void;
  onFixHosts: () => void;
  onToast: (message: string | null) => void;
  onXdebug: (enabled: boolean) => void;
  onTerminal: () => void;
  onRefresh: () => void;
  onMobilePreview: () => void;
  onDeleted: (warnings: string[]) => void;
}) {
  const site = details.site;
  const siteUrl = siteOpenUrl(site, stack);
  const [restorePath, setRestorePath] = useState("");
  const [importPath, setImportPath] = useState("");
  const [srFrom, setSrFrom] = useState("");
  const [srTo, setSrTo] = useState("");
  const [backupsDir, setBackupsDir] = useState<string | null>(null);
  const [showDelete, setShowDelete] = useState(false);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [removeFromDevstack, setRemoveFromDevstack] = useState(true);
  const [dropDatabase, setDropDatabase] = useState(false);
  const [deleteFolder, setDeleteFolder] = useState(false);
  const webServer = (site.webServer ?? "nginx").toLowerCase();
  const dbName = details.databaseName ?? site.databaseName ?? null;
  const dbUser = details.databaseUser ?? site.databaseUser ?? dbName;

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
            onClick={() => onOpenInBrowser()}
          >
            Open in browser
          </button>
        </div>
      </div>

      {details.reachability?.message &&
      (details.reachability.state === "error" ||
        details.reachability.state === "warning") ? (
        <div
          className="error-banner"
          role="status"
          data-testid="site-detail-reachability"
          style={{ marginBottom: 12 }}
        >
          {details.reachability.message}
        </div>
      ) : null}

      <div className="site-detail-grid">
        <div className="panel-card settings-list">
          <div className="settings-row" style={{ background: "#fafafa" }}>
            <label>Environment</label>
          </div>
          <div className="settings-row settings-row-prominent" data-testid="site-php-row">
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
          <div className="settings-row" data-testid="site-xdebug-row">
            <label>
              Xdebug
              <span className="hint">
                Step debugging for this site (port 9003)
              </span>
            </label>
            <button
              type="button"
              role="switch"
              aria-checked={Boolean(site.xdebug)}
              className={`toggle ${site.xdebug ? "on" : ""}`}
              disabled={busy}
              onClick={() => onXdebug(!site.xdebug)}
            />
          </div>
          {site.xdebug ? (
            <div className="settings-row settings-row-block">
              <XdebugHints siteId={site.id} enabled={Boolean(site.xdebug)} />
            </div>
          ) : null}
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
              Terminal
              <span className="hint">
                Shell in the site folder with this site's PHP, Composer and
                Node on PATH
              </span>
            </label>
            <button
              type="button"
              className="btn btn-install"
              onClick={onTerminal}
            >
              Open terminal
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
          {details.hasWpConfig ? (
            <div className="settings-row">
              <label>
                Site URL
                <span className="hint">
                  Sync WordPress siteurl/home to {siteUrl}
                </span>
              </label>
              <button
                type="button"
                className="btn btn-install"
                disabled={busy}
                data-testid="repair-site-url"
                onClick={() =>
                  runOpen(async () => {
                    const msg = await client.repairSiteUrl(site.id);
                    onToast(msg);
                  })
                }
              >
                Repair site URL
              </button>
            </div>
          ) : null}
        </div>
      </div>

      <div className="panel-card settings-list" style={{ marginTop: 16 }}>
        <div className="settings-row" style={{ background: "#fafafa" }}>
          <label>Templates &amp; backups</label>
        </div>
        <div className="settings-row">
          <label>
            Starter template &amp; clone
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
              Clone / Duplicate…
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

      <div className="panel-card settings-list" style={{ marginTop: 16 }}>
        <div className="settings-row">
          <label>
            Mobile preview
            <span className="hint">Open this site on a phone over Wi-Fi (QR code)</span>
          </label>
          <button
            type="button"
            className="btn btn-install"
            disabled={busy}
            onClick={onMobilePreview}
          >
            Preview on phone
          </button>
        </div>
      </div>

      <SiteDomainPanel site={site} onChanged={onRefresh} />
      <SitePhpIniPanel siteId={site.id} />
      <SiteLogsPanel siteId={site.id} />
      <SiteBackupsPanel siteId={site.id} />

      <div className="danger-zone" style={{ marginTop: 16 }}>
        <button
          type="button"
          className="danger-btn"
          disabled={busy || deleteBusy}
          data-testid="delete-application"
          onClick={() => {
            setRemoveFromDevstack(true);
            setDropDatabase(false);
            setDeleteFolder(false);
            setShowDelete(true);
          }}
        >
          Delete Application
        </button>
      </div>

      {showDelete ? (
        <div className="modal-backdrop" role="dialog" aria-modal="true">
          <div className="modal-card" data-testid="delete-site-modal">
            <h3>Delete {site.name}?</h3>
            <p className="hint" style={{ marginTop: 0 }}>
              Choose what to remove. Parked folders are only deleted from disk when
              that box is checked.
            </p>
            <label className="field" style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
              <input
                type="checkbox"
                checked={removeFromDevstack}
                data-testid="delete-opt-devstack"
                onChange={(e) => setRemoveFromDevstack(e.target.checked)}
              />
              <span>
                Remove from DevStack
                <span className="hint">
                  vhosts, hosts entries, certs, per-site php.ini and logs under the
                  DevStack data dir
                </span>
              </span>
            </label>
            <label className="field" style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
              <input
                type="checkbox"
                checked={dropDatabase}
                disabled={!dbName}
                data-testid="delete-opt-database"
                onChange={(e) => setDropDatabase(e.target.checked)}
              />
              <span>
                Also drop database + DB user
                <span className="hint">
                  {dbName
                    ? `${dbName} / ${dbUser ?? "—"}`
                    : "No database recorded for this site"}
                </span>
              </span>
            </label>
            <label className="field" style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
              <input
                type="checkbox"
                checked={deleteFolder}
                data-testid="delete-opt-folder"
                onChange={(e) => setDeleteFolder(e.target.checked)}
              />
              <span>
                Also delete the site folder from disk
                <span className="hint" style={{ wordBreak: "break-all" }}>
                  {details.absolutePath}
                  {site.parked ? " (parked)" : ""}
                </span>
              </span>
            </label>
            <div className="modal-actions">
              <button
                type="button"
                className="btn btn-install"
                disabled={deleteBusy}
                onClick={() => setShowDelete(false)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn btn-primary danger-btn"
                disabled={deleteBusy || !removeFromDevstack}
                data-testid="delete-site-confirm"
                onClick={() => {
                  setDeleteBusy(true);
                  void client
                    .deleteSite(site.id, {
                      removeFromDevstack,
                      dropDatabase: Boolean(dbName) && dropDatabase,
                      deleteFolder,
                    })
                    .then((r) => {
                      setShowDelete(false);
                      onDeleted(r.warnings ?? []);
                    })
                    .catch((e) =>
                      onToast(e instanceof Error ? e.message : String(e)),
                    )
                    .finally(() => setDeleteBusy(false));
                }}
              >
                {deleteBusy ? "Deleting…" : "Delete"}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
