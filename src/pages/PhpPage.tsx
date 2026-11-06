import { useEffect, useState } from "react";
import { Hero, VersionTable } from "../components/VersionTable";
import { usePackageVersions } from "../hooks/usePackageVersions";
import * as client from "../client";
import type { PhpExtensionView } from "../types";

export function PhpPage() {
  const pkg = usePackageVersions("php");
  const [ext, setExt] = useState<PhpExtensionView[]>([]);

  useEffect(() => {
    void client.listPhpExtensionSettings().then(setExt).catch(() => setExt([]));
  }, [pkg.installed]);

  const toggleXdebug = async (version: string, enabled: boolean) => {
    const updated = await client.setPhpXdebug(version, enabled);
    setExt((prev) => {
      const rest = prev.filter((p) => p.version !== version);
      return [...rest, updated];
    });
  };

  return (
    <div>
      <Hero
        title="PHP"
        description="Install and switch between PHP versions, and manage the PHP configuration used across your sites."
        color="#777BB4"
        glyph="P"
      />
      <div className="section-head">
        <h2>Available Versions</h2>
        <p>Install, update and manage PHP.</p>
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
          onInstall={pkg.install}
          onUninstall={pkg.uninstall}
        />
      )}

      {pkg.installed.length > 0 ? (
        <div className="stack" style={{ marginTop: 22 }}>
          <div className="section-head">
            <h2>Extensions</h2>
            <p>Toggle Xdebug per installed PHP version (rewrites php.ini).</p>
          </div>
          <div className="panel-card settings-list">
            {pkg.installed.map((i) => {
              const on = ext.find((e) => e.version === i.version)?.xdebug ?? false;
              return (
                <div className="settings-row" key={i.version}>
                  <label>
                    Xdebug · PHP {i.version}
                    <span className="hint">Debug / develop mode via trigger</span>
                  </label>
                  <button
                    type="button"
                    className={`toggle ${on ? "on" : ""}`}
                    aria-pressed={on}
                    onClick={() => void toggleXdebug(i.version, !on)}
                  />
                </div>
              );
            })}
          </div>
        </div>
      ) : null}
    </div>
  );
}
