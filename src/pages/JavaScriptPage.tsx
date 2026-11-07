import { Hero, VersionTable } from "../components/VersionTable";
import { usePackageVersions } from "../hooks/usePackageVersions";

export function JavaScriptPage() {
  const pkg = usePackageVersions("node");

  return (
    <div>
      <Hero
        title="JavaScript"
        description="Install and switch between Node.js versions from the official Node.js distributions — no nvm required."
        color="#3C873A"
        glyph="JS"
      />
      <div className="section-head">
        <h2>Node.js Versions</h2>
        <p>Install and Update Node.js</p>
      </div>
      {pkg.error ? <div className="error-banner">{pkg.error}</div> : null}
      {pkg.loading && pkg.versions.length === 0 ? (
        <div className="loading">Loading versions…</div>
      ) : (
        <>
          {pkg.refreshing ? (
            <div className="loading">Refreshing version list…</div>
          ) : null}
          <VersionTable
            versions={pkg.versions}
            installed={pkg.installed}
            busyKey={pkg.busyKey}
            progress={pkg.progress}
            onInstall={pkg.install}
            onUninstall={pkg.uninstall}
          />
        </>
      )}
    </div>
  );
}
