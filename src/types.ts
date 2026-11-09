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
  | "performance"
  | "hosting"
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
  /** Default TLD for new sites (`test`). */
  defaultTld?: string;
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
  memcached?: number;
  meilisearch?: number;
  typesense?: number;
  minioApi?: number;
  minioConsole?: number;
  reverb?: number;
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
  /** Top-level domain (`test` by default). */
  tld?: string;
  /** Extra hostnames served by this site. */
  aliases?: string[];
  /** Also serve `*.{domain}`. */
  wildcard?: boolean;
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

// --- v0.3.0 ---------------------------------------------------------------------

export type NewServiceId =
  | "memcached"
  | "meilisearch"
  | "typesense"
  | "minio"
  | "reverb";

export interface PhpIniKnownKey {
  key: string;
  kind: "size" | "int" | "bool" | "text";
  description: string;
}

export interface PhpIniSliders {
  /** `null` = unlimited (-1). */
  memoryLimitMb: number | null;
  /** `0` = unlimited. */
  maxExecutionTime: number;
  uploadMaxMb: number;
  postMaxMb: number;
}

export interface SitePhpIniView {
  siteId: string;
  phpVersion?: string | null;
  iniPath: string;
  mergedIniPath: string;
  overrides: Record<string, string>;
  global: Record<string, string>;
  merged: Record<string, string>;
  sliders: PhpIniSliders;
  knownKeys: PhpIniKnownKey[];
  reloaded: boolean;
  notes: string[];
}

export type SiteTemplateKind =
  | "laravel"
  | "symfony"
  | "statamic"
  | "drupal"
  | "joomla"
  | "woocommerce";

export interface SiteTemplateInfo {
  kind: SiteTemplateKind;
  label: string;
  docroot: string;
  commands: string[];
}

export interface CreateFromTemplateRequest {
  kind: SiteTemplateKind;
  name: string;
  path?: string | null;
  phpVersion?: string | null;
  databaseEngine?: string | null;
  webServer?: string | null;
}

export interface TemplateStepResult {
  name: string;
  ok: boolean;
  skipped: boolean;
  message: string;
}

export interface CreateFromTemplateResult {
  site: SiteRecord;
  kind: string;
  projectPath: string;
  docroot: string;
  url: string;
  database?: {
    engine: string;
    name: string;
    user: string;
    password: string;
    host: string;
    port: number;
  } | null;
  steps: TemplateStepResult[];
  softFailed: boolean;
  notes: string[];
}

export interface HostingRemoteDb {
  host: string;
  name: string;
  user: string;
}

export interface HostingProfile {
  id: string;
  name: string;
  host: string;
  port: number;
  user: string;
  auth: "key" | "password";
  keyPath?: string | null;
  remotePath: string;
  remoteUrl?: string | null;
  siteId?: string | null;
  excludes: string[];
  remoteDb?: HostingRemoteDb | null;
  hasPassword?: boolean;
  hasDbPassword?: boolean;
}

export interface HostingTools {
  ssh: boolean;
  sftp: boolean;
  scp: boolean;
  rsync: boolean;
  keychain: boolean;
  notes: string[];
}

export type SyncDirection = "pull" | "push";

export interface SyncPlanEntry {
  path: string;
  action: "add" | "update" | "delete";
  size: number;
  isDir: boolean;
}

export interface SyncPlan {
  profileId: string;
  direction: SyncDirection;
  dryRun: boolean;
  entries: SyncPlanEntry[];
  add: number;
  update: number;
  delete: number;
  bytes: number;
  excludes: string[];
  tool: string;
  notes: string[];
}

export interface SyncResult {
  profileId: string;
  direction: SyncDirection;
  filesTransferred: number;
  deleted: number;
  bytes: number;
  databaseSynced: boolean;
  urlRewritten?: string | null;
  notes: string[];
}

export interface TldSettings {
  defaultTld: string;
  warnings: string[];
  httpsRequired: boolean;
}

export interface SiteDomainUpdate {
  site: SiteRecord;
  warnings: string[];
  hostsOk: boolean;
  hostsMessage?: string | null;
}

export type SiteLogSource = "php" | "web-error" | "web-access" | "laravel";

export interface LogSourceInfo {
  id: SiteLogSource;
  label: string;
  path: string;
  exists: boolean;
  size: number;
}

export interface SiteLogQuery {
  source: SiteLogSource;
  lines?: number;
  level?: string;
  text?: string;
  /** Absolute timestamp or relative like `15m`, `2h`. */
  since?: string;
  cursor?: number | null;
}

export interface SiteLogEntry {
  ts?: string | null;
  level: string;
  message: string;
}

export interface SiteLogPage {
  source: string;
  path: string;
  entries: SiteLogEntry[];
  cursor: number;
  size: number;
  reset: boolean;
}

export interface QueueStatus {
  isLaravel: boolean;
  driver?: string | null;
  pending?: number | null;
  failed?: number | null;
  failedJobs: { id: string; queue: string; failedAt: string; exception: string }[];
  horizon: { installed: boolean; url?: string | null };
  notes: string[];
}

export interface MobilePreviewInfo {
  active: boolean;
  siteId?: string | null;
  lanIp?: string | null;
  port?: number | null;
  url?: string | null;
  qrDataUrl?: string | null;
  hostHeader?: string | null;
  wpRewritten: boolean;
  firewallNotice?: string | null;
  notes: string[];
}

export interface ProcessMetric {
  id: string;
  label: string;
  pid: number;
  running: boolean;
  cpuPercent: number;
  memoryBytes: number;
  processes: number;
}

export interface MetricsSnapshot {
  timestampMs: number;
  system: {
    cpuPercent: number;
    cpuCount: number;
    totalMemoryBytes: number;
    usedMemoryBytes: number;
  };
  processes: ProcessMetric[];
  totalCpuPercent: number;
  totalMemoryBytes: number;
}

export interface SlowQuery {
  time?: string | null;
  userHost?: string | null;
  queryTime: number;
  lockTime: number;
  rowsSent: number;
  rowsExamined: number;
  sql: string;
}

export interface SlowQueryInfo {
  engine?: string | null;
  running: boolean;
  enabled: boolean;
  thresholdSecs: number;
  path: string;
  exists: boolean;
  entries: SlowQuery[];
  notes: string[];
}

export interface SpxStatus {
  phpVersion?: string | null;
  available: boolean;
  enabled: boolean;
  reportDir: string;
  uiQuery: string;
  notes: string[];
}

export interface ConfigArchiveMeta {
  format: string;
  formatVersion: number;
  appVersion: string;
  createdAt: string;
  os: string;
  siteCount: number;
  siteIds: string[];
  includesDatabases: boolean;
  databases: string[];
  siteMetaFiles: number;
  hasHostingProfiles: boolean;
}

export interface ConfigExportSummary {
  path: string;
  bytes: number;
  meta: ConfigArchiveMeta;
  warnings: string[];
}

export interface ConfigImportOptions {
  replaceSites?: boolean;
  restoreSettings?: boolean;
  restoreDatabases?: boolean;
}

export interface ConfigImportReport {
  sitesAdded: number;
  sitesUpdated: number;
  settingsApplied: boolean;
  siteMetaFiles: number;
  databasesRestored: string[];
  databasesSkipped: string[];
  missingPaths: string[];
  installedPackages: Record<string, string[]>;
  notes: string[];
}
