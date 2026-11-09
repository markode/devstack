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
