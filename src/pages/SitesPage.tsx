import { useEffect, useState } from "react";
import * as client from "../client";
import type { PackageVersion, SiteDetails, StackStatus } from "../types";

export function SitesPage({ initialSiteId }: { initialSiteId?: string | null }) {
  const [details, setDetails] = useState<SiteDetails[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(
    initialSiteId ?? null,
  );
  const [phpVersions, setPhpVersions] = useState<PackageVersion[]>([]);
  const [nodeVersions, setNodeVersions] = useState<PackageVersion[]>([]);
  const [stack, setStack] = useState<StackStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = async () => {
    try {
      const [d, php, node, st] = await Promise.all([
        client.listSiteDetails(),
        client.listPackageVersions("php"),
        client.listPackageVersions("node"),
        client.getStackStatus(),
      ]);
      setDetails(d);
      setPhpVersions(php);
      setNodeVersions(node);
      setStack(st);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    void refresh();
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

  if (selected) {
    return (
      <SiteDetail
        details={selected}
        phpVersions={phpVersions}
        nodeVersions={nodeVersions}
        onBack={backToGrid}
        onPatch={(u) => void patch(selected.site.id, u)}
      />
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
            className="btn btn-primary"
            disabled={busy}
            onClick={() => void toggleStack()}
          >
            {stack?.nginx ? "Stop stack" : "Start stack"}
          </button>
        </div>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
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
            <div className="site-tile-name">{d.site.name}</div>
            <div className="site-tile-domain">{d.site.domain}</div>
            <div className="site-tile-meta">
              <span>PHP {d.site.phpVersion ?? "—"}</span>
              <span className="muted">{d.framework}</span>
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}

function SiteDetail({
  details,
  phpVersions,
  nodeVersions,
  onBack,
  onPatch,
}: {
  details: SiteDetails;
  phpVersions: PackageVersion[];
  nodeVersions: PackageVersion[];
  onBack: () => void;
  onPatch: (u: {
    phpVersion?: string | null;
    nodeVersion?: string | null;
    https?: boolean;
  }) => void;
}) {
  const site = details.site;
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
            <strong>{site.name}</strong>
            <div className="muted" style={{ fontSize: 12 }}>
              {site.domain}
            </div>
          </div>
        </div>
        <div className="row-actions">
          <a
            className="btn btn-primary"
            href={site.https ? details.httpsUrl : details.url}
            target="_blank"
            rel="noreferrer"
          >
            Open in browser
          </a>
        </div>
      </div>

      <div className="site-detail-grid">
        <div className="panel-card settings-list">
          <div className="settings-row" style={{ background: "#fafafa" }}>
            <label>Environment</label>
          </div>
          <div className="settings-row">
            <label>PHP</label>
            <select
              className="native"
              value={site.phpVersion ?? ""}
              onChange={(e) => onPatch({ phpVersion: e.target.value || null })}
            >
              <option value="">System</option>
              {phpVersions.map((v) => (
                <option key={v.version} value={v.channel}>
                  {v.channel}
                </option>
              ))}
            </select>
          </div>
          <div className="settings-row">
            <label>Node</label>
            <select
              className="native"
              value={site.nodeVersion ?? ""}
              onChange={(e) => onPatch({ nodeVersion: e.target.value || null })}
            >
              <option value="">System</option>
              {nodeVersions.map((v) => (
                <option key={v.version} value={v.channel}>
                  {v.channel}
                </option>
              ))}
            </select>
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
              onClick={() => {
                void navigator.clipboard?.writeText(details.absolutePath);
              }}
            >
              Open folder
            </button>
          </div>
          <div className="settings-row">
            <label>
              Editor
              <span className="hint">Reveal path for your IDE</span>
            </label>
            <button
              type="button"
              className="btn btn-install"
              onClick={() => {
                void navigator.clipboard?.writeText(details.absolutePath);
              }}
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
              <a
                className="btn btn-primary"
                href={details.phpmyadminUrl}
                target="_blank"
                rel="noreferrer"
              >
                Open database
              </a>
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

      <div className="danger-zone" style={{ marginTop: 16 }}>
        <button type="button" className="danger-btn" disabled>
          Delete Application
        </button>
      </div>
    </div>
  );
}
