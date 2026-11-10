import { useEffect, useState } from "react";
import { SectionHeader } from "../components/SectionHeader";
import { VersionTable } from "../components/VersionTable";
import { PortConflictDialog } from "../components/PortConflictDialog";
import { usePackageVersions } from "../hooks/usePackageVersions";
import * as client from "../client";
import type {
  ManagedServiceStatus,
  PackageInfo,
  PortCheckResult,
  SiteRecord,
} from "../types";
import { openInBrowser } from "../open";
import { humanizeError, parsePortBusy } from "../portBusy";
import { formatServicePorts } from "../v030";

const SERVICE_IDS = [
  "nginx",
  "apache",
  "phpmyadmin",
  "mysql",
  "mariadb",
  "mongodb",
  "postgresql",
  "redis",
  "mailpit",
  "memcached",
  "meilisearch",
  "typesense",
  "minio",
] as const;

export function ServicesPage() {
  const [packages, setPackages] = useState<PackageInfo[]>([]);
  const [services, setServices] = useState<ManagedServiceStatus[]>([]);
  const [logId, setLogId] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [portConflict, setPortConflict] = useState<PortCheckResult | null>(
    null,
  );

  const refresh = async () => {
    // Load independently so a services-status failure cannot leave Install
    // buttons stuck disabled (meta.installReady was defaulting to false).
    const results = await Promise.allSettled([
      client.listPackages(),
      client.listManagedServices(),
    ]);
    if (results[0].status === "fulfilled") {
      setPackages(results[0].value);
    } else {
      setError(
        results[0].reason instanceof Error
          ? results[0].reason.message
          : String(results[0].reason),
      );
    }
    if (results[1].status === "fulfilled") {
      setServices(results[1].value);
    } else if (results[0].status === "fulfilled") {
      setError(
        results[1].reason instanceof Error
          ? results[1].reason.message
          : String(results[1].reason),
      );
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const toggle = async (svc: ManagedServiceStatus) => {
    setBusyId(svc.id);
    setError(null);
    setPortConflict(null);
    try {
      if (svc.running) {
        await client.stopManagedService(svc.id);
      } else {
        const check = await client.checkServicePorts(svc.id);
        if (!check.ok) {
          setPortConflict(check);
          setError(check.message ?? `Port(s) for ${svc.name} are in use`);
          return;
        }
        await client.startManagedService(svc.id);
      }
      await refresh();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const busy = parsePortBusy(msg);
      if (busy) {
        setPortConflict(busy);
        setError(busy.message ?? humanizeError(msg));
      } else {
        setError(humanizeError(msg));
      }
    } finally {
      setBusyId(null);
    }
  };

  const useNextFreePort = async () => {
    if (!portConflict?.serviceId) return;
    setBusyId(portConflict.serviceId);
    setError(null);
    try {
      await client.applyNextFreePorts(portConflict.serviceId);
      setPortConflict(null);
      await client.startManagedService(portConflict.serviceId);
      await refresh();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const busy = parsePortBusy(msg);
      if (busy) {
        setPortConflict(busy);
        setError(busy.message ?? humanizeError(msg));
      } else {
        setError(humanizeError(msg));
      }
    } finally {
      setBusyId(null);
    }
  };

  const useCoexistence = async () => {
    if (!portConflict?.serviceId) return;
    setBusyId(portConflict.serviceId);
    setError(null);
    try {
      await client.setCoexistenceMode(true);
      setPortConflict(null);
      await client.startManagedService(portConflict.serviceId);
      await refresh();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const busy = parsePortBusy(msg);
      if (busy) {
        setPortConflict(busy);
        setError(busy.message ?? humanizeError(msg));
      } else {
        setError(humanizeError(msg));
      }
    } finally {
      setBusyId(null);
    }
  };

  const showLog = async (id: string) => {
    setLogId(id);
    try {
      setLogText(await client.getServiceLog(id, 120));
    } catch (e) {
      setLogText(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div>
      <SectionHeader
        title="Runtime status"
        description="Ports, process state, and recent logs for each managed service."
      />
      {error ? <div className="error-banner">{error}</div> : null}
      {services
        .filter((svc) => svc.available === false)
        .map((svc) => (
          <div key={`unavail-${svc.id}`} className="port-notice-banner" role="status">
            {svc.name}: {svc.notes ?? "Not available on this platform."}
          </div>
        ))}
      {portConflict && !portConflict.ok ? (
        <PortConflictDialog
          conflict={portConflict}
          busy={!!busyId}
          onCancel={() => setPortConflict(null)}
          onUseSuggested={() => void useNextFreePort()}
          onCoexistence={() => void useCoexistence()}
        />
      ) : null}

      <div className="svc-status-grid">
        {services.map((svc) => (
          <div key={svc.id} className="svc-card">
            <div className="svc-card-top">
              <div>
                <strong>{svc.name}</strong>
                <div className="muted" style={{ fontSize: 12 }}>
                  {svc.version ? `v${svc.version}` : "not installed"}
                  {svc.ports.length ? ` · ${formatServicePorts(svc)}` : ""}
                </div>
              </div>
              <span className={`status-dot ${svc.running ? "on" : ""}`} />
            </div>
            {svc.notes ? (
              <div className="muted" style={{ fontSize: 12, marginTop: 6 }}>
                {svc.notes}
              </div>
            ) : null}
            {svc.id === "reverb" ? <ReverbSitePicker onChanged={() => void refresh()} /> : null}
            <div className="svc-card-actions">
              <button
                type="button"
                className={`btn ${svc.running ? "btn-install" : "btn-primary"}`}
                disabled={
                  busyId === svc.id ||
                  !svc.installed ||
                  svc.available === false
                }
                title={
                  svc.available === false
                    ? (svc.notes ?? "Not available on this platform")
                    : undefined
                }
                onClick={() => void toggle(svc)}
              >
                {svc.running
                  ? "Stop"
                  : svc.available === false
                    ? "Unavailable"
                    : "Start"}
              </button>
              <button
                type="button"
                className="btn btn-install"
                onClick={() => void showLog(svc.id)}
              >
                Logs
              </button>
              {svc.uiUrl ? (
                <button
                  type="button"
                  className="btn btn-install"
                  onClick={() => {
                    void openInBrowser(svc.uiUrl!).catch((e) =>
                      setError(e instanceof Error ? e.message : String(e)),
                    );
                  }}
                >
                  Open
                </button>
              ) : null}
            </div>
          </div>
        ))}
      </div>

      {logId ? (
        <div className="panel-card" style={{ marginBottom: 22 }}>
          <div className="settings-row" style={{ background: "#fafafa" }}>
            <label>Logs · {logId}</label>
            <button
              type="button"
              className="btn btn-install"
              onClick={() => setLogId(null)}
            >
              Close
            </button>
          </div>
          <pre className="svc-log">{logText || "(empty)"}</pre>
        </div>
      ) : null}

      {SERVICE_IDS.map((id) => (
        <ServiceBlock
          key={id}
          id={id}
          meta={packages.find((p) => p.id === id)}
        />
      ))}
    </div>
  );
}

function ReverbSitePicker({ onChanged }: { onChanged: () => void }) {
  const [sites, setSites] = useState<SiteRecord[]>([]);
  const [value, setValue] = useState("");
  useEffect(() => {
    void client.getSites().then(setSites).catch(() => {});
  }, []);
  const laravel = sites.filter((s) => (s.framework ?? "") === "laravel" || (s.framework ?? "") === "statamic");
  return (
    <label className="field" style={{ marginTop: 8 }}>
      <span className="muted" style={{ fontSize: 12 }}>
        Laravel site (needs <code>laravel/reverb</code>)
      </span>
      <select
        className="native"
        aria-label="Reverb site"
        value={value}
        onChange={(e) => {
          setValue(e.target.value);
          void client.setReverbSite(e.target.value || null).then(onChanged);
        }}
      >
        <option value="">Auto-detect</option>
        {laravel.map((s) => (
          <option key={s.id} value={s.id}>
            {s.name}
          </option>
        ))}
      </select>
    </label>
  );
}

function ServiceBlock({
  id,
  meta,
}: {
  id: string;
  meta?: PackageInfo;
}) {
  const pkg = usePackageVersions(id);
  const [localError, setLocalError] = useState<string | null>(null);
  // Default true so a missing/slow package-meta fetch does not grey out Install.
  const installReady = meta?.installReady ?? true;
  return (
    <div style={{ marginBottom: 22 }}>
      <div className="section-head">
        <h2>
          {meta?.name ?? id}
          {meta && !meta.installReady ? (
            <span className="badge">list only</span>
          ) : null}
        </h2>
        <p>{meta?.description ?? "Service package"}</p>
      </div>
      {pkg.error || localError ? (
        <div className="error-banner">{pkg.error ?? localError}</div>
      ) : null}
      {pkg.loading && pkg.versions.length === 0 ? (
        <div className="loading">Loading versions…</div>
      ) : pkg.error && pkg.versions.length === 0 ? null : (
        <>
          {pkg.refreshing ? (
            <div className="loading">Refreshing version list…</div>
          ) : null}
          <VersionTable
            versions={pkg.versions}
            installed={pkg.installed}
            busyKey={pkg.busyKey}
            progress={pkg.progress}
            installReady={installReady}
            onInstall={(version) => {
              setLocalError(null);
              void pkg.install(version);
            }}
            onUninstall={pkg.uninstall}
            onUnavailable={(message) => setLocalError(message)}
          />
        </>
      )}
    </div>
  );
}
