export type SectionId =
  | "general"
  | "php"
  | "javascript"
  | "services"
  | "sites"
  | "logs"
  | "ai"
  | "pro"
  | "sharing"
  | "mail"
  | "dumps"
  | "backups"
  | "shortcuts"
  | "about";

export type LogLevel = "info" | "warn" | "error";

export interface LogEntry {
  id: number;
  ts: string;
  level: LogLevel;
  source: string;
  message: string;
}

export interface DeveloperSettings {
  showLogsTab: boolean;
  redactSecrets: boolean;
}

export interface PackageInfo {
  id: string;
  name: string;
  description: string;
  category: string;
  installReady: boolean;
}

export interface PackageVersion {
  version: string;
  channel: string;
  label: string;
  lts: boolean;
  downloadUrl?: string | null;
  sha256?: string | null;
  notes?: string | null;
}

export interface VersionListResponse {
  packageId: string;
  versions: PackageVersion[];
  fromCache: boolean;
  refreshing: boolean;
  fetchedAt?: string | null;
  warning?: string | null;
}

export interface VersionsUpdatedEvent {
  packageId: string;
  versions: PackageVersion[];
  fetchedAt: string;
  error?: string | null;
}

export interface HostsStatus {
  ok: boolean;
  missing: string[];
  message?: string | null;
}

export interface InstalledPackage {
  packageId: string;
  version: string;
  channel?: string | null;
  path: string;
  installedAt: string;
}

export interface ProgressEvent {
  packageId: string;
  version: string;
  phase: string;
  downloaded: number;
  total?: number | null;
}

export interface StackStatus {
  nginx: boolean;
  apache: boolean;
  dns: boolean;
  database: boolean;
  databaseEngine?: string | null;
  httpPort: number;
  httpsPort: number;
  apacheHttpPort: number;
  apacheHttpsPort: number;
  mysqlPort: number;
  mariadbPort: number;
  postgresqlPort: number;
  mongodbPort: number;
  phpFpmPort: number;
  coexistenceMode: boolean;
  phpVersions: string[];
  /** Banner lines when ports were auto-remapped beside WAMP/MAMP. */
  portNotice?: string[];
}

export interface SiteReachability {
  /** `running` | `stopped` | `error` | `warning` */
  state: string;
  message?: string | null;
  /** Fresh log lines for a Show log expander. */
  logExcerpt?: string | null;
  tcpOk: boolean;
  httpOk: boolean;
  probedUrl: string;
  probedPort: number;
}

export interface PortsConfig {
  coexistenceMode: boolean;
  nginxHttp: number;
  nginxHttps: number;
  apacheHttp: number;
  apacheHttps: number;
  mysql: number;
  mariadb: number;
  postgresql: number;
  mongodb: number;
  phpFpm: number;
  redis: number;
  mailpitSmtp: number;
  mailpitUi: number;
  dns: number;
  phpmyadmin: number;
}

export interface PortOccupant {
  port: number;
  pid?: number | null;
  processName?: string | null;
  hint?: string | null;
  windowsService?: string | null;
}

export interface PortCheckResult {
  serviceId: string;
  ok: boolean;
  conflicts: PortOccupant[];
  message?: string | null;
  suggested: Record<string, number>;
}

export interface AppInfo {
  name: string;
  version: string;
  dataDir: string;
  os: string;
  arch: string;
  phpManifestUrl?: string | null;
  stack?: StackStatus;
  developer?: DeveloperSettings;
}

export interface DatabaseFilesSettings {
  folder?: string | null;
  autoExportIntervalSecs: number;
  autoExportOnStop: boolean;
  importMode: string;
}

export interface DbFileEntry {
  engine: string;
  database: string;
  path: string;
  bytes: number;
  modifiedAt?: string | null;
}

export interface SiteRecord {
  id: string;
  name: string;
  path: string;
  domain: string;
  phpVersion?: string | null;
  nodeVersion?: string | null;
  https: boolean;
  /** `"nginx"` | `"apache"` — which front-end serves this site. */
  webServer?: string | null;
  framework?: string | null;
  databaseEngine?: string | null;
  databaseName?: string | null;
  databaseUser?: string | null;
  databasePassword?: string | null;
  status?: string;
  isTemplate?: boolean;
  /** Xdebug enabled for this site's PHP-FPM pool. */
  xdebug?: boolean;
  /** True when the site was created by a parked directory scan. */
  parked?: boolean;
}

export interface SiteDetails {
  site: SiteRecord;
  framework: string;
  frameworkIcon: string;
  absolutePath: string;
  url: string;
  httpsUrl: string;
  databaseName?: string | null;
  databaseUser?: string | null;
  phpmyadminUrl?: string | null;
  hasWpConfig: boolean;
  reachability?: SiteReachability | null;
}

export interface JobProgress {
  jobId: string;
  phase: string;
  message: string;
  bytes: number;
  total?: number | null;
  percent?: number | null;
}

export interface TemplateRecord {
  id: string;
  name: string;
  sourceSiteId: string;
  sourceDomain: string;
  sourceUrl: string;
  framework: string;
  createdAt: string;
  filesArchive: string;
  dbArchive?: string | null;
  sizeBytes: number;
  databaseEngine?: string | null;
}

export interface CreateWordPressRequest {
  name: string;
  domain?: string | null;
  path?: string | null;
  phpVersion?: string | null;
  databaseEngine?: string | null;
  locale?: string | null;
  adminUser?: string | null;
  siteTitle?: string | null;
}

export interface CreateWordPressResult {
  site: SiteRecord;
  adminUser: string;
  adminPassword: string;
  adminEmail: string;
  url: string;
  wordpressVersion: string;
}

export interface DuplicateRequest {
  sourceId: string;
  name: string;
  domain?: string | null;
  path: string;
  phpVersion?: string | null;
  databaseEngine?: string | null;
}

export interface DuplicateResult {
  site: SiteRecord;
  url: string;
}

export interface BackupResult {
  path: string;
  bytes: number;
}

export interface ManagedServiceStatus {
  id: string;
  name: string;
  running: boolean;
  installed: boolean;
  version?: string | null;
  ports: number[];
  logPath?: string | null;
  uiUrl?: string | null;
  notes?: string | null;
}

export interface PhpExtensionView {
  version: string;
  xdebug: boolean;
  iniPath?: string | null;
}

export interface SitesSettings {
  parkedDirs: string[];
  watchEnabled: boolean;
  optInPath: boolean;
}

export interface SitesSettingsPatch {
  parkedDirs?: string[];
  watchEnabled?: boolean;
  optInPath?: boolean;
}

export interface ParkedSyncResult {
  added: number;
  sites: SiteRecord[];
}

export interface ShareStatus {
  active: boolean;
  siteId?: string | null;
  url?: string | null;
  pid?: number | null;
}

export interface PhpDump {
  id: number;
  /** Unix epoch milliseconds. */
  ts: number;
  /** Site id or domain that emitted the dump. */
  site?: string | null;
  /** JSON payload captured by dump(). */
  payload: unknown;
  file?: string | null;
  line?: number | null;
}

export interface SiteBackup {
  id: string;
  siteId: string;
  siteName: string;
  domain: string;
  createdAt: string;
  dir: string;
  filesArchive: string;
  dbArchive?: string | null;
  dbName?: string | null;
  dbEngine?: string | null;
  sizeBytes: number;
  warnings: string[];
}

export interface BackupSettings {
  folder?: string | null;
  /** Auto-backup every N seconds (0 = off). */
  intervalSecs: number;
  /** Keep the newest N backups per site. */
  retain: number;
}

export interface BackupSettingsPatch {
  /** Empty string resets to the default folder. */
  folder?: string | null;
  intervalSecs?: number;
  retain?: number;
}

export interface PhpStormHint {
  serverName: string;
  host: string;
  port: number;
  localPath: string;
  remotePath: string;
  ideKey: string;
}

/** Result of `set_site_xdebug` / `get_xdebug_launch_hints`. */
export interface XdebugLaunchHints {
  siteId: string;
  phpVersion: string;
  enabled: boolean;
  port: number;
  /** Path of the `.vscode/launch.json` the backend wrote, if any. */
  launchJson?: string | null;
  phpstorm: PhpStormHint;
}

export interface ToolsStatus {
  binDir: string;
  composer: boolean;
  wpCli: boolean;
  node: boolean;
  shims: boolean;
  optInPath: boolean;
}

export interface DumpStatus {
  port?: number | null;
  helperPath: string;
}
