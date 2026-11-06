import { useEffect, useState } from "react";
import { Hero, VersionTable } from "../components/VersionTable";
import { usePackageVersions } from "../hooks/usePackageVersions";
import * as client from "../client";
import type { ManagedServiceStatus, PackageInfo } from "../types";

const SERVICE_IDS = [
  "nginx",
  "phpmyadmin",
  "mysql",
  "mariadb",
  "mongodb",
  "postgresql",
  "redis",
  "mailpit",
] as const;

export function ServicesPage() {
  const [packages, setPackages] = useState<PackageInfo[]>([]);
  const [services, setServices] = useState<ManagedServiceStatus[]>([]);
  const [logId, setLogId] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const [pkgs, svc] = await Promise.all([
        client.listPackages(),
        client.listManagedServices(),
      ]);
      setPackages(pkgs);
      setServices(svc);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const toggle = async (svc: ManagedServiceStatus) => {
    setBusyId(svc.id);
    setError(null);
    try {
      if (svc.running) {
        await client.stopManagedService(svc.id);
      } else {
        await client.startManagedService(svc.id);
      }
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
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
      <Hero
        title="Services"
        description="Install runtimes from official sources, then start/stop with live ports and logs."
        color="#5856d6"
        glyph="Sv"
      />
      {error ? <div className="error-banner">{error}</div> : null}

      <div className="section-head">
        <h2>Runtime status</h2>
        <p>Ports, process state, and recent stderr for each managed service.</p>
      </div>
      <div className="svc-status-grid">
        {services.map((svc) => (
          <div key={svc.id} className="svc-card">
            <div className="svc-card-top">
              <div>
                <strong>{svc.name}</strong>
                <div className="muted" style={{ fontSize: 12 }}>
                  {svc.version ? `v${svc.version}` : "not installed"}
                  {svc.ports.length
                    ? ` · ${svc.ports.map((p) => `:${p}`).join(" ")}`
                    : ""}
                </div>
              </div>
              <span className={`status-dot ${svc.running ? "on" : ""}`} />
            </div>
            {svc.notes ? (
              <div className="muted" style={{ fontSize: 12, marginTop: 6 }}>
                {svc.notes}
              </div>
            ) : null}
            <div className="svc-card-actions">
              <button
                type="button"
                className={`btn ${svc.running ? "btn-install" : "btn-primary"}`}
                disabled={busyId === svc.id || !svc.installed}
                onClick={() => void toggle(svc)}
              >
                {svc.running ? "Stop" : "Start"}
              </button>
              <button
                type="button"
                className="btn btn-install"
                onClick={() => void showLog(svc.id)}
              >
                Logs
              </button>
              {svc.uiUrl ? (
                <a
                  className="btn btn-install"
                  href={svc.uiUrl}
                  target="_blank"
                  rel="noreferrer"
                >
                  Open
                </a>
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

function ServiceBlock({
  id,
  meta,
}: {
  id: string;
  meta?: PackageInfo;
}) {
  const pkg = usePackageVersions(id);
  return (
    <div style={{ marginBottom: 22 }}>
      <div className="section-head">
        <h2>
          {meta?.name ?? id}
          {!meta?.installReady ? (
            <span className="badge">list only</span>
          ) : null}
        </h2>
        <p>{meta?.description ?? "Service package"}</p>
      </div>
      {pkg.error ? <div className="error-banner">{pkg.error}</div> : null}
      {pkg.loading ? (
        <div className="loading">Loading versions…</div>
      ) : (
        <VersionTable
          versions={pkg.versions}
          installed={pkg.installed}
          busyKey={pkg.busyKey}
          progress={pkg.progress}
          installReady={meta?.installReady ?? false}
          onInstall={pkg.install}
          onUninstall={pkg.uninstall}
        />
      )}
    </div>
  );
}
