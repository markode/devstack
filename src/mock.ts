import type {
  AppInfo,
  BackupResult,
  BackupSettings,
  BackupSettingsPatch,
  ParkedSyncResult,
  PhpDump,
  ShareStatus,
  SiteBackup,
  SitesSettings,
  ToolsStatus,
  XdebugLaunchHints,
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
  ManagedServiceStatus,
  TemplateRecord,
  VersionListResponse,
} from "./types";
import type {
  ConfigArchiveMeta,
  ConfigExportSummary,
  ConfigImportReport,
  CreateFromTemplateRequest,
  CreateFromTemplateResult,
  HostingProfile,
  HostingTools,
  LogSourceInfo,
  MetricsSnapshot,
  MobilePreviewInfo,
  PhpIniSliders,
  QueueStatus,
  SiteDomainUpdate,
  SiteLogPage,
  SiteLogQuery,
  SitePhpIniView,
  SiteTemplateInfo,
  SlowQueryInfo,
  SpxStatus,
  SyncDirection,
  SyncPlan,
  SyncResult,
  TldSettings,
} from "./types";

const installed = new Map<string, InstalledPackage[]>();

export const mockStackStatus: StackStatus = {
  nginx: true,
  apache: true,
  dns: true,
  database: true,
  databaseEngine: "mariadb",
  httpPort: 80,
  httpsPort: 443,
  apacheHttpPort: 8088,
  apacheHttpsPort: 8444,
  mysqlPort: 3306,
  mariadbPort: 3306,
  postgresqlPort: 5432,
  mongodbPort: 27017,
  phpFpmPort: 9000,
  coexistenceMode: false,
  phpVersions: ["8.5.1"],
};

let mockPorts: import("./types").PortsConfig = {
  coexistenceMode: false,
  nginxHttp: 80,
  nginxHttps: 443,
  apacheHttp: 8088,
  apacheHttps: 8444,
  mysql: 3306,
  mariadb: 3306,
  postgresql: 5432,
  mongodb: 27017,
  phpFpm: 9000,
  redis: 6379,
  mailpitSmtp: 1025,
  mailpitUi: 8025,
  dns: 5353,
  phpmyadmin: 443,
};

export function mockPortsConfig() {
  return { ...mockPorts };
}

export function mockUpdatePortsConfig(config: import("./types").PortsConfig) {
  mockPorts = { ...config, phpmyadmin: config.nginxHttps };
  return { ...mockPorts };
}

export function mockSetCoexistenceMode(enabled: boolean) {
  if (enabled) {
    mockPorts = {
      coexistenceMode: true,
      nginxHttp: 8080,
      nginxHttps: 8443,
      apacheHttp: 8088,
      apacheHttps: 8444,
      mysql: 3307,
      mariadb: 3308,
      postgresql: 5433,
      mongodb: 27018,
      phpFpm: 9001,
      redis: 6380,
      mailpitSmtp: 1026,
      mailpitUi: 8026,
      dns: 5354,
      phpmyadmin: 8443,
    };
  } else {
    mockPorts = {
      coexistenceMode: false,
      nginxHttp: 80,
      nginxHttps: 443,
      apacheHttp: 8088,
      apacheHttps: 8444,
      mysql: 3306,
      mariadb: 3306,
      postgresql: 5432,
      mongodb: 27017,
      phpFpm: 9000,
      redis: 6379,
      mailpitSmtp: 1025,
      mailpitUi: 8025,
      dns: 5353,
      phpmyadmin: 443,
    };
  }
  return { ...mockPorts };
}

export function mockCheckServicePorts(serviceId: string) {
  return {
    serviceId,
    ok: true,
    conflicts: [],
    message: null,
    suggested: {},
  };
}

export function mockApplyNextFreePorts(serviceId: string) {
  if (serviceId === "nginx") {
    mockPorts.nginxHttp = 8080;
    mockPorts.nginxHttps = 8443;
    mockPorts.phpmyadmin = 8443;
    mockPorts.coexistenceMode = true;
  }
  return { ...mockPorts };
}

export const mockLogState = {
  errorCount: 0,
  ring: [] as import("./types").LogEntry[],
  nextId: 1,
};
export const mockDeveloperSettingsState = {
  showLogsTab: true,
  redactSecrets: false,
};

export function mockAppendAppLog(
  level: import("./types").LogLevel,
  source: string,
  message: string,
) {
  if (level === "error") mockLogState.errorCount += 1;
  mockLogState.ring.push({
    id: mockLogState.nextId++,
    ts: new Date().toISOString(),
    level,
    source,
    message,
  });
  if (mockLogState.ring.length > 5000) mockLogState.ring.shift();
}

export function mockGetAppLogs(limit?: number) {
  const n = limit ?? mockLogState.ring.length;
  return mockLogState.ring.slice(-n);
}

export function mockClearAppLogs() {
  mockLogState.ring.length = 0;
  mockLogState.errorCount = 0;
  mockAppendAppLog("info", "app", "log buffer cleared");
}

export const mockDeveloperSettings = mockDeveloperSettingsState;

export const mockAppInfo: AppInfo = {
  name: "DevStack",
  version: "0.1.0",
  dataDir: "/Users/dev/Library/Application Support/DevStack",
  os: "macos",
  arch: "aarch64",
  phpManifestUrl: null,
  stack: mockStackStatus,
  developer: mockDeveloperSettingsState,
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
    id: "apache",
    name: "Apache",
    description: "Apache HTTP Server — full .htaccess / mod_rewrite for WordPress.",
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
    installReady: true,
  },
  {
    id: "postgresql",
    name: "PostgreSQL",
    description: "PostgreSQL.",
    category: "service",
    installReady: true,
  },
  {
    id: "redis",
    name: "Redis",
    description: "Redis in-memory store.",
    category: "service",
    installReady: true,
  },
  {
    id: "mailpit",
    name: "Mailpit",
    description: "Catch-all SMTP + web UI.",
    category: "service",
    installReady: true,
  },
];

let managed: ManagedServiceStatus[] = [
  {
    id: "nginx",
    name: "Nginx",
    running: true,
    installed: true,
    version: "1.28.3",
    ports: [80, 443],
    logPath: "/tmp/nginx.err",
  },
  {
    id: "apache",
    name: "Apache",
    running: true,
    installed: true,
    version: "2.4.66",
    ports: [8088, 8444],
    logPath: "/tmp/apache.err",
    notes: "HTTP :8088 · HTTPS :8444 (alongside Nginx)",
  },
  {
    id: "mariadb",
    name: "MariaDB",
    running: true,
    installed: true,
    version: "11.4.5",
    ports: [3306],
  },
  {
    id: "mongodb",
    name: "MongoDB",
    running: false,
    installed: true,
    version: "8.0.4",
    ports: [27017],
  },
  {
    id: "postgresql",
    name: "PostgreSQL",
    running: true,
    installed: true,
    version: "16.10.0",
    ports: [5432],
  },
  {
    id: "redis",
    name: "Redis",
    running: true,
    installed: true,
    version: "8.0.4",
    ports: [6379],
  },
  {
    id: "mailpit",
    name: "Mailpit",
    running: true,
    installed: true,
    version: "1.22.3",
    ports: [1025, 8025],
    uiUrl: "https://mailpit.test",
    notes: "SMTP :1025 · UI :8025 / mailpit.test",
  },
  {
    id: "memcached",
    name: "Memcached",
    running: false,
    installed: true,
    version: "1.6.21",
    ports: [11211],
    notes: "TCP :11211 — cache / session backend",
  },
  {
    id: "meilisearch",
    name: "Meilisearch",
    running: false,
    installed: true,
    version: "1.15.2",
    ports: [7700],
    uiUrl: "http://127.0.0.1:7700",
    notes: "HTTP :7700 · dev mode, no master key",
  },
  {
    id: "typesense",
    name: "Typesense",
    running: false,
    installed: false,
    version: null,
    ports: [8108],
    notes: "HTTP :8108 · API key `devstack` · macOS/Linux only",
  },
  {
    id: "minio",
    name: "MinIO",
    running: false,
    installed: true,
    version: "2026-10-26T00-00-00Z",
    ports: [9100, 9101],
    uiUrl: "http://127.0.0.1:9101",
    notes: "S3 API :9100 · console :9101 · login devstack / devstack123",
  },
  {
    id: "reverb",
    name: "Laravel Reverb",
    running: false,
    installed: true,
    version: null,
    ports: [8085],
    notes: "WebSocket :8085 · runs `php artisan reverb:start` in acme-dashboard.test",
  },
];

mockPackages.push(
  ...(
    [
      ["memcached", "Memcached", "In-memory cache (uses the memcached on PATH)."],
      ["meilisearch", "Meilisearch", "Typo-tolerant search engine on :7700."],
      ["typesense", "Typesense", "Fast in-memory search engine (macOS/Linux)."],
      ["minio", "MinIO", "S3-compatible object storage with web console."],
    ] as const
  ).map(([id, name, description]) => ({
    id,
    name,
    description,
    category: "service",
    installReady: true,
  })),
);

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
  apache: [
    {
      version: "2.4.66",
      channel: "2.4",
      label: "2.4.66 (Apache Lounge VS17 x64)",
      lts: true,
      downloadUrl:
        "https://www.apachelounge.com/download/VS17/binaries/httpd-2.4.66-251206-Win64-VS17.zip",
      notes: "Official Apache Lounge Win64 VS17 build",
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
  postgresql: ["16.10.0", "15.5.4"].map((v) => ({
    version: v,
    channel: v.split(".")[0],
    label: v,
    lts: true,
    downloadUrl: "https://example.invalid/pg.tar.gz",
  })),
  mongodb: ["8.0.4", "7.0.14"].map((v) => ({
    version: v,
    channel: v.split(".").slice(0, 2).join("."),
    label: v,
    lts: true,
    downloadUrl: "https://example.invalid/mongo.tgz",
  })),
  redis: ["8.0.4", "7.4.2"].map((v) => ({
    version: v,
    channel: v.split(".").slice(0, 2).join("."),
    label: v,
    lts: true,
    downloadUrl: "https://example.invalid/redis.tgz",
  })),
  mailpit: ["1.22.3", "1.21.0"].map((v) => ({
    version: v,
    channel: v,
    label: v,
    lts: true,
    downloadUrl: "https://example.invalid/mailpit.tgz",
  })),
  memcached: [
    { version: "1.6.21", channel: "1.6.21", label: "Memcached 1.6.21 (system)", lts: true },
  ],
  meilisearch: ["1.15.2", "1.12.8"].map((v) => ({
    version: v,
    channel: v.split(".").slice(0, 2).join("."),
    label: `Meilisearch ${v}`,
    lts: true,
    downloadUrl: "https://example.invalid/meilisearch",
  })),
  typesense: ["29.0", "27.1"].map((v) => ({
    version: v,
    channel: v,
    label: `Typesense ${v}`,
    lts: true,
    downloadUrl: "https://example.invalid/typesense.tgz",
  })),
  minio: [
    {
      version: "2026-10-26T00-00-00Z",
      channel: "2026-10",
      label: "MinIO 2026-10-26",
      lts: true,
      downloadUrl: "https://example.invalid/minio.tgz",
    },
  ],
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
    webServer: "nginx",
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
    webServer: "apache",
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
    webServer: "nginx",
    framework: "other",
    status: "stopped",
    isTemplate: false,
  },
];

let templates: TemplateRecord[] = [];

const phpExt = new Map<string, boolean>();

export async function mockListVersions(id: string): Promise<VersionListResponse> {
  await delay(120);
  return {
    packageId: id,
    versions: versionTables[id] ?? [],
    fromCache: false,
    refreshing: false,
    fetchedAt: new Date().toISOString(),
    warning: null,
  };
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
  const apache = (s.webServer ?? "").toLowerCase() === "apache";
  const httpPort = apache ? mockStackStatus.apacheHttpPort : mockStackStatus.httpPort;
  const httpsPort = apache
    ? mockStackStatus.apacheHttpsPort
    : mockStackStatus.httpsPort;
  return {
    site: s,
    framework,
    frameworkIcon: icon,
    absolutePath: s.path.replace("~", "/Users/dev"),
    url:
      httpPort === 80
        ? `http://${s.domain}`
        : `http://${s.domain}:${httpPort}`,
    httpsUrl:
      httpsPort === 443
        ? `https://${s.domain}`
        : `https://${s.domain}:${httpsPort}`,
    databaseName: s.databaseName,
    databaseUser: s.databaseUser,
    phpmyadminUrl:
      framework === "wordpress" && s.databaseName
        ? `https://phpmyadmin.test/devstack-signon.php?db=${s.databaseName}`
        : null,
    hasWpConfig: framework === "wordpress",
  };
}

export async function mockListSiteDetails(): Promise<SiteDetails[]> {
  return sites.map(toDetails);
}

/** Simulated slow probe pass — used to assert cache-first UI. */
export async function mockListSiteDetailsProbed(): Promise<SiteDetails[]> {
  await delay(30);
  return sites.map((s) => {
    const d = toDetails(s);
    return {
      ...d,
      reachability: {
        state: s.status === "running" ? "running" : "stopped",
        message: null,
        tcpOk: s.status === "running",
        httpOk: s.status === "running",
        probedUrl: d.url,
        probedPort: 80,
      },
    };
  });
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
seedInstalled("apache", "2.4.66", "2.4");
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
    webServer: "apache",
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
    url: `https://${site.domain}:8444`,
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
    webServer: "apache",
    framework: "wordpress",
    databaseEngine: request.databaseEngine || "mariadb",
    databaseName: `wp_${id.replace(/-/g, "_")}`,
    databaseUser: `u_${id.replace(/-/g, "_")}`,
    status: "running",
    isTemplate: false,
  };
  sites = [...sites, site];
  return { site, url: `https://${site.domain}:8444` };
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

export async function mockManagedServices(): Promise<ManagedServiceStatus[]> {
  return managed;
}

export async function mockStartService(
  serviceId: string,
): Promise<ManagedServiceStatus> {
  await delay(200);
  managed = managed.map((s) =>
    s.id === serviceId ? { ...s, running: true } : s,
  );
  return managed.find((s) => s.id === serviceId)!;
}

export async function mockStopService(
  serviceId: string,
): Promise<ManagedServiceStatus> {
  await delay(200);
  managed = managed.map((s) =>
    s.id === serviceId ? { ...s, running: false } : s,
  );
  return managed.find((s) => s.id === serviceId)!;
}

export async function mockServiceLog(serviceId: string): Promise<string> {
  return `[devstack] mock log for ${serviceId}\nready for connections`;
}

let mockSitesSettings: SitesSettings = {
  parkedDirs: [],
  watchEnabled: true,
  optInPath: false,
};

export function mockGetSitesSettings(): SitesSettings {
  return { ...mockSitesSettings, parkedDirs: [...mockSitesSettings.parkedDirs] };
}

export function mockUpdateSitesSettings(patch: {
  parkedDirs?: string[];
  watchEnabled?: boolean;
  optInPath?: boolean;
}): SitesSettings {
  mockSitesSettings = {
    parkedDirs: patch.parkedDirs ?? mockSitesSettings.parkedDirs,
    watchEnabled: patch.watchEnabled ?? mockSitesSettings.watchEnabled,
    optInPath: patch.optInPath ?? mockSitesSettings.optInPath,
  };
  return mockGetSitesSettings();
}

export function mockSyncParkedSites(): ParkedSyncResult {
  return { added: 0, sites };
}

let mockShare: ShareStatus = { active: false };

export function mockGetShareStatus(): ShareStatus {
  return { ...mockShare };
}

export async function mockStartShare(siteId: string): Promise<ShareStatus> {
  await delay(150);
  mockShare = {
    active: true,
    siteId,
    url: `https://${siteId}-demo.trycloudflare.com`,
    pid: 4242,
  };
  return { ...mockShare };
}

export function mockStopShare(): ShareStatus {
  mockShare = { active: false };
  return { ...mockShare };
}

let mockDumps: PhpDump[] = [];

export function mockListDumps(limit?: number): PhpDump[] {
  return limit ? mockDumps.slice(-limit) : [...mockDumps];
}

export function mockClearDumps(): void {
  mockDumps = [];
}

let mockBackups: SiteBackup[] = [];
let mockBackupSettings: BackupSettings = {
  folder: null,
  intervalSecs: 0,
  retain: 10,
};

export function mockListBackups(siteId?: string): SiteBackup[] {
  return mockBackups.filter((b) => !siteId || b.siteId === siteId);
}

export async function mockCreateBackup(siteId: string): Promise<SiteBackup> {
  await delay(200);
  const site = sites.find((s) => s.id === siteId);
  const stamp = Date.now();
  const dir = `/tmp/DevStack/backups/${siteId}/${stamp}`;
  const backup: SiteBackup = {
    id: `bk-${stamp}`,
    siteId,
    siteName: site?.name ?? siteId,
    domain: site?.domain ?? `${siteId}.test`,
    createdAt: new Date().toISOString(),
    dir,
    filesArchive: `${dir}/files.tar.zst`,
    dbArchive: site?.databaseName ? `${dir}/db.sql.zst` : null,
    dbName: site?.databaseName ?? null,
    dbEngine: site?.databaseEngine ?? null,
    sizeBytes: 12_400_000,
    warnings: [],
  };
  mockBackups = [backup, ...mockBackups];
  return backup;
}

export async function mockRestoreBackup(backupId: string): Promise<SiteBackup> {
  await delay(200);
  return mockBackups.find((b) => b.id === backupId) ?? mockBackups[0];
}

export function mockDeleteBackup(backupId: string): void {
  mockBackups = mockBackups.filter((b) => b.id !== backupId);
}

export function mockGetBackupSettings(): BackupSettings {
  return { ...mockBackupSettings };
}

export function mockUpdateBackupSettings(
  patch: BackupSettingsPatch,
): BackupSettings {
  mockBackupSettings = {
    folder:
      patch.folder === undefined
        ? mockBackupSettings.folder
        : patch.folder || null,
    intervalSecs: patch.intervalSecs ?? mockBackupSettings.intervalSecs,
    retain: patch.retain ?? mockBackupSettings.retain,
  };
  return { ...mockBackupSettings };
}

export function mockXdebugHints(siteId: string): XdebugLaunchHints {
  const site = sites.find((s) => s.id === siteId);
  const root = site?.path.replace("~", "/Users/dev") ?? "/Users/dev/Sites/app";
  return {
    siteId,
    phpVersion: site?.phpVersion ?? "8.5",
    enabled: Boolean(site?.xdebug),
    port: 9003,
    launchJson: `${root}/.vscode/launch.json`,
    phpstorm: {
      serverName: site?.domain ?? `${siteId}.test`,
      host: site?.domain ?? `${siteId}.test`,
      port: site?.https === false ? 80 : 443,
      localPath: root,
      remotePath: root,
      ideKey: "PHPSTORM",
    },
  };
}

export async function mockSetSiteXdebug(
  siteId: string,
  enabled: boolean,
): Promise<XdebugLaunchHints> {
  await mockUpdateSite(siteId, { xdebug: enabled });
  return mockXdebugHints(siteId);
}

export function mockToolsStatus(): ToolsStatus {
  return {
    binDir: "/Users/dev/Library/Application Support/DevStack/tools/bin",
    composer: true,
    wpCli: true,
    node: true,
    shims: true,
    optInPath: false,
  };
}

// --- v0.3.0 mocks ---------------------------------------------------------------

export async function mockSetReverbSite(siteId: string | null): Promise<ManagedServiceStatus> {
  const svc = managed.find((s) => s.id === "reverb")!;
  const site = sites.find((x) => x.id === siteId);
  svc.notes = site
    ? `WebSocket :8085 · runs \`php artisan reverb:start\` in ${site.domain}`
    : "Select a Laravel site with laravel/reverb installed";
  return svc;
}

const phpIniOverrides = new Map<string, Record<string, string>>();
const GLOBAL_INI: Record<string, string> = {
  memory_limit: "-1",
  max_execution_time: "0",
  upload_max_filesize: "0",
  post_max_size: "0",
};

function iniView(siteId: string): SitePhpIniView {
  const overrides = phpIniOverrides.get(siteId) ?? {};
  const merged = { ...GLOBAL_INI, ...overrides };
  const mb = (v?: string) => {
    if (!v || v === "-1" || v === "0") return null;
    const n = parseInt(v, 10);
    return /g$/i.test(v) ? n * 1024 : n;
  };
  return {
    siteId,
    phpVersion: "8.3.6",
    iniPath: `/tmp/DevStack/sites/${siteId}/php.ini`,
    mergedIniPath: `/tmp/DevStack/sites/${siteId}/php-merged.ini`,
    overrides,
    global: GLOBAL_INI,
    merged,
    sliders: {
      memoryLimitMb: mb(merged.memory_limit),
      maxExecutionTime: parseInt(merged.max_execution_time, 10) || 0,
      uploadMaxMb: mb(merged.upload_max_filesize) ?? 0,
      postMaxMb: mb(merged.post_max_size) ?? 0,
    },
    knownKeys: [
      { key: "memory_limit", kind: "size", description: "Max memory per request (-1 = unlimited)" },
      { key: "max_execution_time", kind: "int", description: "Seconds a script may run (0 = unlimited)" },
      { key: "upload_max_filesize", kind: "size", description: "Largest single upload" },
      { key: "post_max_size", kind: "size", description: "Largest POST body" },
      { key: "display_errors", kind: "bool", description: "Show errors in output" },
      { key: "date.timezone", kind: "text", description: "Default timezone" },
    ],
    reloaded: true,
    notes: [],
  };
}

export async function mockGetSitePhpIni(siteId: string): Promise<SitePhpIniView> {
  return iniView(siteId);
}

export async function mockSetSitePhpIni(
  siteId: string,
  overrides: Record<string, string>,
): Promise<SitePhpIniView> {
  await delay(40);
  phpIniOverrides.set(siteId, { ...overrides });
  return iniView(siteId);
}

export async function mockSetSitePhpIniSliders(
  siteId: string,
  s: PhpIniSliders,
): Promise<SitePhpIniView> {
  const next = {
    ...(phpIniOverrides.get(siteId) ?? {}),
    memory_limit: s.memoryLimitMb == null ? "-1" : `${s.memoryLimitMb}M`,
    max_execution_time: String(s.maxExecutionTime),
    upload_max_filesize: s.uploadMaxMb ? `${s.uploadMaxMb}M` : "0",
    post_max_size: s.postMaxMb ? `${s.postMaxMb}M` : "0",
  };
  return mockSetSitePhpIni(siteId, next);
}

export async function mockSiteTemplateKinds(): Promise<SiteTemplateInfo[]> {
  return [
    ["laravel", "Laravel", "public", ["composer create-project laravel/laravel <dir>"]],
    ["symfony", "Symfony", "public", ["composer create-project symfony/skeleton <dir>"]],
    ["statamic", "Statamic", "public", ["composer create-project statamic/statamic <dir>"]],
    ["drupal", "Drupal", "web", ["composer create-project drupal/recommended-project <dir>"]],
    ["joomla", "Joomla", "", ["download Joomla full package and unzip"]],
    ["woocommerce", "WooCommerce", "", ["wp core install", "wp plugin install woocommerce --activate"]],
  ].map(([kind, label, docroot, commands]) => ({
    kind: kind as SiteTemplateInfo["kind"],
    label: label as string,
    docroot: docroot as string,
    commands: commands as string[],
  }));
}

export async function mockCreateFromTemplate(
  req: CreateFromTemplateRequest,
): Promise<CreateFromTemplateResult> {
  await delay(120);
  const id = req.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "site";
  const site: SiteRecord = {
    id,
    name: req.name,
    path: `~/Sites/${id}/public`,
    domain: `${id}.test`,
    phpVersion: req.phpVersion ?? "8.3",
    https: true,
    webServer: req.kind === "woocommerce" ? "apache" : "nginx",
    framework: req.kind === "woocommerce" ? "wordpress" : req.kind,
    databaseEngine: "mariadb",
    databaseName: `${req.kind}_${id.replace(/-/g, "_")}`,
    status: "stopped",
  };
  sites = [...sites.filter((s) => s.id !== id), site];
  return {
    site,
    kind: req.kind,
    projectPath: `~/Sites/${id}`,
    docroot: site.path,
    url: `https://${site.domain}`,
    database: {
      engine: "mariadb",
      name: site.databaseName!,
      user: `u_${id.replace(/-/g, "_")}`,
      password: "mock-password",
      host: "127.0.0.1",
      port: 3306,
    },
    steps: [
      { name: "database", ok: true, skipped: false, message: "created database" },
      { name: "project", ok: true, skipped: false, message: "composer create-project finished" },
      { name: "env", ok: true, skipped: false, message: "database credentials written to .env" },
    ],
    softFailed: false,
    notes: [],
  };
}

let hostingProfiles: HostingProfile[] = [];

export async function mockHostingTools(): Promise<HostingTools> {
  return { ssh: true, sftp: true, scp: true, rsync: true, keychain: true, notes: [] };
}

export async function mockHostingProfiles(): Promise<HostingProfile[]> {
  return hostingProfiles;
}

export async function mockHostingSave(
  profile: HostingProfile,
  password?: string | null,
): Promise<HostingProfile> {
  const id = profile.id || `${profile.name}-${profile.host}`.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  const saved = { ...profile, id, port: profile.port || 22, hasPassword: !!password };
  hostingProfiles = [...hostingProfiles.filter((p) => p.id !== id), saved];
  return saved;
}

export async function mockHostingDelete(id: string): Promise<void> {
  hostingProfiles = hostingProfiles.filter((p) => p.id !== id);
}

export async function mockHostingPreview(
  profileId: string,
  direction: SyncDirection,
): Promise<SyncPlan> {
  const entries = [
    { path: "index.php", action: "update" as const, size: 420, isDir: false },
    { path: "wp-content/uploads/hero.jpg", action: "add" as const, size: 183_000, isDir: false },
    { path: "old-backup.zip", action: "delete" as const, size: 0, isDir: false },
  ];
  return {
    profileId,
    direction,
    dryRun: true,
    entries,
    add: 1,
    update: 1,
    delete: 1,
    bytes: 183_420,
    excludes: [".git/", "*.log"],
    tool: "rsync",
    notes: [],
  };
}

export async function mockHostingRun(
  profileId: string,
  direction: SyncDirection,
  includeDatabase: boolean,
): Promise<SyncResult> {
  await delay(80);
  return {
    profileId,
    direction,
    filesTransferred: 2,
    deleted: 0,
    bytes: 183_420,
    databaseSynced: includeDatabase,
    urlRewritten: includeDatabase ? "blog.test: https://example.com → https://blog.test" : null,
    notes: [],
  };
}

let defaultTld = "test";

function tldWarnings(tld: string): string[] {
  const w: string[] = [];
  if (tld === "dev" || tld === "app") w.push(`.${tld} is HSTS-preloaded: HTTPS is forced.`);
  if (tld === "local") w.push(".local is reserved for mDNS and can be slow.");
  return w;
}

export async function mockTldSettings(): Promise<TldSettings> {
  return { defaultTld, warnings: tldWarnings(defaultTld), httpsRequired: ["dev", "app"].includes(defaultTld) };
}

export async function mockSetDefaultTld(tld: string): Promise<TldSettings> {
  defaultTld = tld.replace(/^\./, "").toLowerCase();
  return mockTldSettings();
}

export async function mockUpdateSiteDomain(
  siteId: string,
  patch: { tld?: string; aliases?: string[]; wildcard?: boolean },
): Promise<SiteDomainUpdate> {
  const cur = sites.find((s) => s.id === siteId);
  if (!cur) throw new Error(`site not found: ${siteId}`);
  const tld = patch.tld?.replace(/^\./, "").toLowerCase();
  const next: SiteRecord = {
    ...cur,
    tld: tld ?? cur.tld ?? "test",
    domain: tld ? `${cur.domain.split(".").slice(0, -1).join(".")}.${tld}` : cur.domain,
    aliases: patch.aliases ?? cur.aliases ?? [],
    wildcard: patch.wildcard ?? cur.wildcard ?? false,
    https: tld === "dev" ? true : cur.https,
  };
  sites = sites.map((s) => (s.id === siteId ? next : s));
  return { site: next, warnings: tld ? tldWarnings(tld) : [], hostsOk: true, hostsMessage: null };
}

export async function mockLogSources(siteId: string): Promise<LogSourceInfo[]> {
  const laravel = sites.find((s) => s.id === siteId)?.framework === "laravel";
  const base: LogSourceInfo[] = [
    { id: "php", label: "PHP errors", path: "/tmp/php.log", exists: true, size: 512 },
    { id: "web-error", label: "Web server errors", path: "/tmp/e.log", exists: true, size: 256 },
    { id: "web-access", label: "Access log", path: "/tmp/a.log", exists: true, size: 4096 },
  ];
  if (laravel) {
    base.push({ id: "laravel", label: "Laravel (storage/logs)", path: "/tmp/laravel.log", exists: true, size: 2048 });
  }
  return base;
}

export async function mockReadSiteLogs(
  _siteId: string,
  q: SiteLogQuery,
): Promise<SiteLogPage> {
  const all = [
    { ts: "2026-11-09 12:00:00", level: "info", message: "Application booted" },
    { ts: "2026-11-09 12:00:04", level: "warning", message: "Slow query (1.4s) on posts" },
    { ts: "2026-11-09 12:00:09", level: "error", message: "Undefined variable $user in /app/Http/Kernel.php" },
  ];
  const order = ["debug", "info", "notice", "warning", "error", "critical"];
  const min = q.level && q.level !== "all" ? order.indexOf(q.level) : 0;
  const text = (q.text ?? "").toLowerCase();
  const entries = all.filter(
    (e) => order.indexOf(e.level) >= min && (!text || e.message.toLowerCase().includes(text)),
  );
  return {
    source: q.source,
    path: "/tmp/mock.log",
    entries: q.cursor ? [] : entries,
    cursor: 1024,
    size: 1024,
    reset: false,
  };
}

export async function mockQueueStatus(siteId: string): Promise<QueueStatus> {
  const laravel = sites.find((s) => s.id === siteId)?.framework === "laravel";
  return {
    isLaravel: laravel,
    driver: laravel ? "database" : null,
    pending: laravel ? 3 : null,
    failed: laravel ? 1 : null,
    failedJobs: laravel
      ? [{ id: "1", queue: "default", failedAt: "2026-11-09 11:00:00", exception: "RuntimeException: boom" }]
      : [],
    horizon: { installed: laravel, url: laravel ? "https://acme-dashboard.test/horizon" : null },
    notes: laravel ? [] : ["Queue panel is available for Laravel sites."],
  };
}

let previewState: MobilePreviewInfo = { active: false, wpRewritten: false, notes: [] };

export async function mockMobilePreview(): Promise<MobilePreviewInfo> {
  return previewState;
}

export async function mockStartMobilePreview(siteId: string): Promise<MobilePreviewInfo> {
  const site = sites.find((s) => s.id === siteId);
  previewState = {
    active: true,
    siteId,
    lanIp: "192.168.1.20",
    port: 8300,
    url: "http://192.168.1.20:8300",
    qrDataUrl:
      "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMCIgaGVpZ2h0PSIxMCI+PHJlY3Qgd2lkdGg9IjEwIiBoZWlnaHQ9IjEwIi8+PC9zdmc+",
    hostHeader: site?.domain ?? null,
    wpRewritten: site?.framework === "wordpress",
    firewallNotice: "If a firewall is active allow the port, e.g. `sudo ufw allow 8300/tcp`.",
    notes: [],
  };
  return previewState;
}

export async function mockStopMobilePreview(): Promise<MobilePreviewInfo> {
  previewState = { active: false, wpRewritten: false, notes: [] };
  return previewState;
}

export async function mockMetricsSnapshot(): Promise<MetricsSnapshot> {
  const jitter = () => Math.round(Math.random() * 40) / 10;
  const processes = [
    { id: "nginx", label: "Nginx", pid: 101, running: true, cpuPercent: jitter(), memoryBytes: 18_000_000, processes: 3 },
    { id: "mariadb", label: "MariaDB", pid: 102, running: true, cpuPercent: jitter(), memoryBytes: 320_000_000, processes: 1 },
    { id: "php-8.3.6", label: "PHP 8.3.6", pid: 103, running: true, cpuPercent: jitter(), memoryBytes: 90_000_000, processes: 3 },
  ];
  return {
    timestampMs: Date.now(),
    system: { cpuPercent: 12.5, cpuCount: 8, totalMemoryBytes: 16 * 1024 ** 3, usedMemoryBytes: 8 * 1024 ** 3 },
    processes,
    totalCpuPercent: processes.reduce((n, p) => n + p.cpuPercent, 0),
    totalMemoryBytes: processes.reduce((n, p) => n + p.memoryBytes, 0),
  };
}

let slowEnabled = false;

export async function mockSlowQueryInfo(): Promise<SlowQueryInfo> {
  return {
    engine: "mariadb",
    running: true,
    enabled: slowEnabled,
    thresholdSecs: 1,
    path: "/tmp/DevStack/logs/mariadb-slow.log",
    exists: true,
    entries: slowEnabled
      ? [
          {
            time: "261109 12:00:00",
            userHost: "wp[wp] @ localhost []",
            queryTime: 2.5,
            lockTime: 0,
            rowsSent: 10,
            rowsExamined: 52000,
            sql: "SELECT * FROM wp_posts WHERE post_status = 'publish'",
          },
        ]
      : [],
    notes: [],
  };
}

export async function mockSetSlowQueryLog(enabled: boolean, thresholdSecs: number): Promise<SlowQueryInfo> {
  slowEnabled = enabled;
  return { ...(await mockSlowQueryInfo()), thresholdSecs };
}

let spxEnabled = false;

export async function mockSpxStatus(): Promise<SpxStatus> {
  return {
    phpVersion: "8.3.6",
    available: false,
    enabled: spxEnabled,
    reportDir: "/tmp/DevStack/spx-reports",
    uiQuery: "?SPX_KEY=dev&SPX_UI_URI=/",
    notes: ["php-spx extension not installed for this PHP."],
  };
}

export async function mockSetSpx(enabled: boolean): Promise<SpxStatus> {
  spxEnabled = enabled;
  return mockSpxStatus();
}

const mockArchiveMeta = (): ConfigArchiveMeta => ({
  format: "devstack-config",
  formatVersion: 1,
  appVersion: "0.3.1",
  createdAt: new Date(0).toISOString(),
  os: "mock",
  siteCount: sites.length,
  siteIds: sites.map((s) => s.id),
  includesDatabases: false,
  databases: [],
  siteMetaFiles: 0,
  hasHostingProfiles: false,
});

export async function mockExportConfig(
  destPath: string,
  _includeDatabases: boolean,
): Promise<ConfigExportSummary> {
  return { path: destPath, bytes: 2048, meta: mockArchiveMeta(), warnings: [] };
}

export async function mockInspectConfig(_path: string): Promise<ConfigArchiveMeta> {
  return mockArchiveMeta();
}

export async function mockImportConfig(_path: string): Promise<ConfigImportReport> {
  return {
    sitesAdded: 0,
    sitesUpdated: sites.length,
    settingsApplied: true,
    siteMetaFiles: 0,
    databasesRestored: [],
    databasesSkipped: [],
    missingPaths: [],
    installedPackages: {},
    notes: [],
  };
}
