import { useEffect, useState } from "react";
import * as client from "../client";
import type { AppInfo } from "../types";

export function AboutPage() {
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    void client.getAppInfo().then(setInfo).catch(() => setInfo(null));
  }, []);

  return (
    <div className="about-page">
      <div className="about-hero">
        <div className="about-mark" aria-hidden>
          <span>DS</span>
        </div>
        <div>
          <h2 className="about-brand">DevStack</h2>
          <p className="about-tagline">
            Local PHP development environment for macOS &amp; Windows —
            Nginx, databases, WordPress, and tooling in one place.
          </p>
        </div>
      </div>

      <div className="about-grid">
        <div className="panel-card about-card">
          <h3>Version</h3>
          <dl className="about-dl">
            <div>
              <dt>App</dt>
              <dd>{info?.version ?? "0.1.0"}</dd>
            </div>
            <div>
              <dt>Platform</dt>
              <dd>
                {info?.os ?? "—"} / {info?.arch ?? "—"}
              </dd>
            </div>
            <div>
              <dt>Data directory</dt>
              <dd className="mono">{info?.dataDir ?? "—"}</dd>
            </div>
          </dl>
        </div>

        <div className="panel-card about-card">
          <h3>Stack</h3>
          <ul className="about-list">
            <li>Tauri 2 · Rust core · React UI</li>
            <li>Nginx · PHP-FPM/CGI · Node</li>
            <li>MySQL / MariaDB · MongoDB · PostgreSQL</li>
            <li>Redis · Mailpit · phpMyAdmin</li>
            <li>.test domains · local HTTPS CA</li>
          </ul>
        </div>

        <div className="panel-card about-card">
          <h3>Updates</h3>
          <p className="muted" style={{ margin: 0, fontSize: 13 }}>
            Auto-updater is configured as a stub. Signed release builds can
            publish update manifests to GitHub Releases; unsigned CI builds
            still produce installers.
          </p>
        </div>
      </div>

      <p className="about-foot muted">
        Inspired by Laravel Herd / MAMP PRO · Proprietary / TBD
      </p>
    </div>
  );
}
