import { useEffect, useState } from "react";
import { Hero, VersionTable } from "../components/VersionTable";
import { usePackageVersions } from "../hooks/usePackageVersions";
import * as client from "../client";
import type { PackageInfo } from "../types";

const SERVICE_IDS = [
  "nginx",
  "phpmyadmin",
  "mysql",
  "mariadb",
  "mongodb",
  "postgresql",
] as const;

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

export function ServicesPage() {
  const [packages, setPackages] = useState<PackageInfo[]>([]);

  useEffect(() => {
    void client.listPackages().then(setPackages).catch(() => setPackages([]));
  }, []);

  return (
    <div>
      <Hero
        title="Services"
        description="Databases and tools use the same package manager as PHP and Node: live version lists, checksums, one-click install."
        color="#5856d6"
        glyph="Sv"
      />
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
