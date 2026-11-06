import type {
  AppInfo,
  BackupResult,
  CreateWordPressRequest,
  CreateWordPressResult,
  DuplicateRequest,
  DuplicateResult,
  InstalledPackage,
  PackageInfo,
  PackageVersion,
  PhpExtensionView,
  SiteDetails,
  SiteRecord,
  StackStatus,
  TemplateRecord,
} from "./types";

const installed = new Map<string, InstalledPackage[]>();

export const mockStackStatus: StackStatus = {
  nginx: true,
  dns: true,
  database: true,
  databaseEngine: "mariadb",
  httpPort: 80,
  httpsPort: 443,
  phpVersions: ["8.5.1"],
};

export const mockAppInfo: AppInfo = {
  name: "DevStack",
  version: "0.1.0",
  dataDir: "/Users/dev/Library/Application Support/DevStack",
  os: "macos",
  arch: "aarch64",
  phpManifestUrl: null,
  stack: mockStackStatus,
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
    id: "nginx",
    name: "Nginx",
    description: "One Nginx for all sites.",
    category: "service",
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
    installReady: true,
  },
  {
    id: "mariadb",
    name: "MariaDB",
    description: "MariaDB Server.",
    category: "service",
    installReady: true,
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
  nginx: [
    {
      version: "1.28.3",
      channel: "1.28",
      label: "1.28.3",
      lts: true,
      downloadUrl: "https://nginx.org/download/nginx-1.28.3.zip",
    },
    {
      version: "1.27.5",
      channel: "1.27",
      label: "1.27.5",
      lts: false,
      downloadUrl: "https://nginx.org/download/nginx-1.27.5.zip",
    },
  ],
  phpmyadmin: [
    {
      version: "5.2.3",
      channel: "5.2",
      label: "5.2.3",
      lts: false,
      notes: "PHP >=7.2,<8.4",
      downloadUrl: "https://files.phpmyadmin.net/phpMyAdmin/5.2.3/phpMyAdmin-5.2.3-all-languages.zip",
    },
  ],
  mysql: ["8.4.5", "8.0.40"].map((v) => ({
    version: v,
    channel: v.split(".").slice(0, 2).join("."),
    label: v,
    lts: true,
    downloadUrl: "https://cdn.mysql.com/example.zip",
  })),
  mariadb: ["11.8.9", "11.4.5"].map((v) => ({
    version: v,
    channel: v.split(".").slice(0, 2).join("."),
    label: v,
    lts: true,
    downloadUrl: "https://downloads.mariadb.org/example.tar.gz",
  })),
  mongodb: ["8.0.4", "7.0.14"].map((v) => ({
    version: v,
    channel: v.split(".").slice(0, 2).join("."),
    label: v,
    lts: true,
  })),
  postgresql: ["17.4", "16.8"].map((v) => ({
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
    domain: "acme-dashboard.test",
    phpVersion: "8.5",
    nodeVersion: "24",
    https: true,
    framework: "laravel",
    databaseEngine: "mariadb",
    databaseName: "acme",
    status: "running",
  },
  {
    id: "blog",
    name: "blog",
    path: "~/Sites/blog",
    domain: "blog.test",
    phpVersion: "8.4",
    nodeVersion: null,
    https: true,
    framework: "wordpress",
    databaseEngine: "mariadb",
    databaseName: "blog",
    databaseUser: "wp",
    status: "running",
    isTemplate: true,
  },
  {
    id: "api-kit",
    name: "api-kit",
    path: "~/Sites/api-kit",
    domain: "api-kit.test",
    phpVersion: "8.3",
    https: false,
    framework: "other",
    status: "stopped",
    isTemplate: false,
  },
];

let templates: TemplateRecord[] = [];

const phpExt = new Map<string, boolean>();

export async function mockListVersions(id: string): Promise<PackageVersion[]> {
  await delay(120);
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
  await delay(400);
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
  await delay(150);
  installed.set(
    packageId,
    (installed.get(packageId) ?? []).filter((x) => x.version !== version),
  );
}

export async function mockGetSites(): Promise<SiteRecord[]> {
  return sites;
}

function toDetails(s: SiteRecord): SiteDetails {
  const framework = s.framework ?? "other";
  const icon =
    framework === "wordpress" ? "WP" : framework === "laravel" ? "LV" : "?";
  return {
    site: s,
    framework,
    frameworkIcon: icon,
    absolutePath: s.path.replace("~", "/Users/dev"),
    url: `http://${s.domain}`,
    httpsUrl: `https://${s.domain}`,
    databaseName: s.databaseName,
    databaseUser: s.databaseUser,
    phpmyadminUrl:
      framework === "wordpress" && s.databaseName
        ? `https://phpmyadmin.test/devstack-signon.php?db=${s.databaseName}&user=${s.databaseUser ?? "root"}&pass=`
        : null,
    hasWpConfig: framework === "wordpress",
  };
}

export async function mockListSiteDetails(): Promise<SiteDetails[]> {
  return sites.map(toDetails);
}

export async function mockGetSiteDetails(siteId: string): Promise<SiteDetails> {
  const s = sites.find((x) => x.id === siteId) ?? sites[0];
  return toDetails(s);
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

seedInstalled("php", "8.5.1", "8.5");
seedInstalled("node", "24.18.0", "24");
seedInstalled("node", "22.23.2", "22");
seedInstalled("nginx", "1.28.3", "1.28");
seedInstalled("mariadb", "11.4.5", "11.4");
seedInstalled("phpmyadmin", "5.2.3", "5.2");

export async function mockCreateWordpress(
  request: CreateWordPressRequest,
): Promise<CreateWordPressResult> {
  await delay(600);
  const id = request.name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  const site: SiteRecord = {
    id,
    name: request.name,
    path: request.path || `~/Sites/${id}`,
    domain: request.domain || `${id}.test`,
    phpVersion: request.phpVersion || "8.4",
    https: true,
    framework: "wordpress",
    databaseEngine: request.databaseEngine || "mariadb",
    databaseName: `wp_${id.replace(/-/g, "_")}`,
    databaseUser: `u_${id.replace(/-/g, "_")}`,
    status: "running",
    isTemplate: false,
  };
  sites = [...sites.filter((s) => s.id !== id), site];
  return {
    site,
    adminUser: request.adminUser || "admin",
    adminPassword: "OnceOnly-Passw0rd!",
    adminEmail: `admin@${site.domain}`,
    url: `https://${site.domain}`,
    wordpressVersion: "6.8.1",
  };
}

export async function mockMarkTemplate(
  siteId: string,
): Promise<TemplateRecord> {
  await delay(400);
  const site = sites.find((s) => s.id === siteId);
  if (!site) throw new Error("site not found");
  sites = sites.map((s) =>
    s.id === siteId ? { ...s, isTemplate: true } : s,
  );
  const tpl: TemplateRecord = {
    id: `${siteId}-tpl`,
    name: site.name,
    sourceSiteId: site.id,
    sourceDomain: site.domain,
    sourceUrl: `https://${site.domain}`,
    framework: site.framework || "wordpress",
    createdAt: new Date().toISOString(),
    filesArchive: `/tmp/templates/${siteId}/files.tar.gz`,
    dbArchive: `/tmp/templates/${siteId}/database.sql.gz`,
    sizeBytes: 12_000_000,
    databaseEngine: site.databaseEngine,
  };
  templates = [...templates.filter((t) => t.id !== tpl.id), tpl];
  return tpl;
}

export async function mockGetTemplates(): Promise<TemplateRecord[]> {
  return templates;
}

export async function mockDuplicateSite(
  request: DuplicateRequest,
): Promise<DuplicateResult> {
  await delay(700);
  const id = request.name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  const site: SiteRecord = {
    id,
    name: request.name,
    path: request.path,
    domain: request.domain || `${id}.test`,
    phpVersion: request.phpVersion || "8.4",
    https: true,
    framework: "wordpress",
    databaseEngine: request.databaseEngine || "mariadb",
    databaseName: `wp_${id.replace(/-/g, "_")}`,
    databaseUser: `u_${id.replace(/-/g, "_")}`,
    status: "running",
    isTemplate: false,
  };
  sites = [...sites, site];
  return { site, url: `https://${site.domain}` };
}

export async function mockBackupSite(siteId: string): Promise<BackupResult> {
  await delay(300);
  return {
    path: `/tmp/backups/${siteId}/db.sql.gz`,
    bytes: 4_200_000,
  };
}

export async function mockRestoreSite(
  _siteId: string,
  _archivePath: string,
): Promise<void> {
  await delay(300);
}
