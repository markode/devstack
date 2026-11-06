import { useEffect, useState } from "react";
import * as client from "../client";
import type { PackageVersion, SiteRecord } from "../types";

export function SitesPage() {
  const [sites, setSites] = useState<SiteRecord[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [phpVersions, setPhpVersions] = useState<PackageVersion[]>([]);
  const [nodeVersions, setNodeVersions] = useState<PackageVersion[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const [s, php, node] = await Promise.all([
          client.getSites(),
          client.listPackageVersions("php"),
          client.listPackageVersions("node"),
        ]);
        setSites(s);
        setSelected(s[0]?.id ?? null);
        setPhpVersions(php);
        setNodeVersions(node);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
  }, []);

  const site = sites.find((s) => s.id === selected) ?? null;

  const patch = async (update: Partial<SiteRecord>) => {
    if (!site) return;
    const next = await client.updateSiteEnvironment(site.id, {
      phpVersion: update.phpVersion,
      nodeVersion: update.nodeVersion,
      https: update.https,
    });
    setSites((prev) => prev.map((s) => (s.id === next.id ? next : s)));
  };

  return (
    <div>
      <div className="section-head" style={{ marginBottom: 16 }}>
        <h2>Sites</h2>
        <p>Per-site PHP / Node versions and HTTPS (environment isolation).</p>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
      <div className="site-layout">
        <div className="site-preview">
          <div className="site-thumb" />
          <h3>{site?.name ?? "No sites"}</h3>
          <button type="button" className="btn btn-primary" style={{ width: "100%" }}>
            Open
          </button>
          <div className="stack" style={{ marginTop: 12 }}>
            {sites.map((s) => (
              <button
                key={s.id}
                type="button"
                className={`nav-item ${selected === s.id ? "active" : ""}`}
                onClick={() => setSelected(s.id)}
              >
                <span className="nav-label">{s.name}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="stack">
          <div className="panel-card settings-list">
            <div className="settings-row" style={{ background: "#fafafa" }}>
              <label>
                Services
                <span className="hint">Linked services come in a later milestone</span>
              </label>
              <button type="button" className="btn btn-danger" style={{ color: "var(--accent)" }}>
                Manage Services…
              </button>
            </div>
          </div>

          <div className="panel-card settings-list">
            <div className="settings-row">
              <label>PHP</label>
              <select
                className="native"
                value={site?.phpVersion ?? ""}
                disabled={!site}
                onChange={(e) =>
                  void patch({ phpVersion: e.target.value || null })
                }
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
                value={site?.nodeVersion ?? ""}
                disabled={!site}
                onChange={(e) =>
                  void patch({ nodeVersion: e.target.value || null })
                }
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
                className={`toggle ${site?.https ? "on" : ""}`}
                disabled={!site}
                onClick={() => void patch({ https: !site?.https })}
              />
            </div>
          </div>

          <div className="danger-zone">
            <button type="button" className="danger-btn" disabled>
              Delete Application
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
