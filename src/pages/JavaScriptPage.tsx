import { SectionHeader } from "../components/SectionHeader";
import { VersionTable } from "../components/VersionTable";
import { usePackageVersions } from "../hooks/usePackageVersions";

export function JavaScriptPage() {
  const pkg = usePackageVersions("node");

  return (
    <div>
      <SectionHeader
        title="Node.js versions"
        description="Install and update Node.js from the official distributions."
      />
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
