import type {
  AppInfo,
  InstalledPackage,
  PackageInfo,
  PackageVersion,
  PhpExtensionView,
  SiteRecord,
} from "./types";

const installed = new Map<string, InstalledPackage[]>();

export const mockAppInfo: AppInfo = {
  name: "DevStack",
  version: "0.1.0",
  dataDir: "/Users/dev/Library/Application Support/DevStack",
  os: "macos",
  arch: "aarch64",
  phpManifestUrl: null,
};

export const mockPackages: PackageInfo[] = [
  {
    id: "php",
    name: "PHP",
    description: "Install and switch between PHP versions.",
    category: "runtime",
    installReady: true,
  },
  {
    id: "node",
    name: "Node.js",
    description: "Install and switch between Node.js versions.",
    category: "runtime",
    installReady: true,
  },
  {
    id: "phpmyadmin",
    name: "phpMyAdmin",
    description: "Web UI for MySQL/MariaDB.",
    category: "service",
    installReady: true,
  },
  {
    id: "mysql",
    name: "MySQL",
    description: "MySQL Community Server.",
    category: "service",
    installReady: false,
  },
  {
    id: "mariadb",
    name: "MariaDB",
    description: "MariaDB Server.",
    category: "service",
    installReady: false,
  },
  {
    id: "mongodb",
    name: "MongoDB",
    description: "MongoDB Community Server.",
    category: "service",
    installReady: false,
  },
  {
    id: "postgresql",
    name: "PostgreSQL",
    description: "PostgreSQL.",
    category: "service",
    installReady: false,
  },
];

const versionTables: Record<string, PackageVersion[]> = {
  php: ["8.5", "8.4", "8.3", "8.2", "8.1", "8.0", "7.4"].map((c, i) => ({
    version: i === 0 ? "8.5.1" : `${c}.0`,
    channel: c,
    label: i < 2 ? `${c} (${c}.${i === 0 ? 1 : 0})` : c,
    lts: false,
    downloadUrl: "https://example.invalid/php.zip",
    sha256: null,
    notes: null,
  })),
  node: [
    { version: "24.18.0", channel: "24", label: "24 (24.18.0)", lts: false },
    { version: "22.23.2", channel: "22", label: "22 (22.23.2)", lts: true },
    { version: "20.19.0", channel: "20", label: "20", lts: true },
    { version: "18.20.0", channel: "18", label: "18", lts: false },
    { version: "16.20.0", channel: "16", label: "16", lts: false },
  ],
  phpmyadmin: [
    {
      version: "5.2.3",
      channel: "5.2",
      label: "5.2.3",
      lts: false,
      notes: "PHP >=7.2,<8.4",
    },
    {
      version: "4.9.11",
      channel: "4.9",
      label: "4.9.11",
      lts: false,
      notes: "Legacy",
    },
  ],
  mysql: ["9.3.0", "8.4.5", "8.0.40"].map((v) => ({
    version: v,
    channel: v.split(".").slice(0, 2).join("."),
    label: v,
    lts: v.startsWith("8."),
  })),
  mariadb: ["11.8.9", "11.4.5", "10.11.10"].map((v) => ({
    version: v,
    channel: v.split(".").slice(0, 2).join("."),
    label: v,
    lts: true,
  })),
  mongodb: ["8.0.4", "7.0.14"].map((v) => ({
    version: v,
    channel: v.split(".").slice(0, 2).join("."),
    label: v,
    lts: true,
  })),
  postgresql: ["17.4", "16.8", "15.12"].map((v) => ({
    version: v,
    channel: v.split(".")[0],
    label: v,
    lts: true,
  })),
};

let sites: SiteRecord[] = [
  {
    id: "acme-dashboard",
    name: "acme-dashboard",
    path: "~/Sites/acme-dashboard",
    phpVersion: "8.5",
    nodeVersion: "24",
    https: true,
  },
];

const phpExt = new Map<string, boolean>();

export async function mockListVersions(id: string): Promise<PackageVersion[]> {
  await delay(180);
  return versionTables[id] ?? [];
}

export async function mockListInstalled(id?: string): Promise<InstalledPackage[]> {
  if (id) return installed.get(id) ?? [];
  return [...installed.values()].flat();
}

export async function mockInstall(
  packageId: string,
  version: string,
): Promise<InstalledPackage> {
  await delay(500);
  const versions = versionTables[packageId] ?? [];
  const meta =
    versions.find((v) => v.version === version || v.channel === version) ??
    versions[0];
  const rec: InstalledPackage = {
    packageId,
    version: meta?.version ?? version,
    channel: meta?.channel,
    path: `/Users/dev/Library/Application Support/DevStack/packages/${packageId}/${meta?.version ?? version}`,
    installedAt: new Date().toISOString(),
  };
  const list = installed.get(packageId) ?? [];
  installed.set(
    packageId,
    [...list.filter((x) => x.version !== rec.version), rec],
  );
  return rec;
}

export async function mockUninstall(
  packageId: string,
  version: string,
): Promise<void> {
  await delay(200);
  installed.set(
    packageId,
    (installed.get(packageId) ?? []).filter((x) => x.version !== version),
  );
}

export async function mockGetSites(): Promise<SiteRecord[]> {
  return sites;
}

export async function mockUpdateSite(
  siteId: string,
  patch: Partial<SiteRecord>,
): Promise<SiteRecord> {
  sites = sites.map((s) => (s.id === siteId ? { ...s, ...patch } : s));
  return sites.find((s) => s.id === siteId)!;
}

export async function mockPhpExt(): Promise<PhpExtensionView[]> {
  return (installed.get("php") ?? []).map((p) => ({
    version: p.version,
    xdebug: phpExt.get(p.version) ?? false,
    iniPath: `${p.path}/php.ini`,
  }));
}

export async function mockSetXdebug(
  version: string,
  enabled: boolean,
): Promise<PhpExtensionView> {
  phpExt.set(version, enabled);
  return { version, xdebug: enabled, iniPath: null };
}

function delay(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

function seedInstalled(packageId: string, version: string, channel: string) {
  const rec: InstalledPackage = {
    packageId,
    version,
    channel,
    path: `/Users/dev/Library/Application Support/DevStack/packages/${packageId}/${version}`,
    installedAt: new Date().toISOString(),
  };
  const list = installed.get(packageId) ?? [];
  installed.set(packageId, [...list.filter((x) => x.version !== version), rec]);
}

// Seed installs so the UI shows Installed states like the Herd screenshots.
seedInstalled("php", "8.5.1", "8.5");
seedInstalled("node", "24.18.0", "24");
seedInstalled("node", "22.23.2", "22");
