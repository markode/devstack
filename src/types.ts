export type SectionId =
  | "general"
  | "php"
  | "javascript"
  | "services"
  | "sites"
  | "ai"
  | "pro"
  | "sharing"
  | "shortcuts"
  | "about";

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
  phpmyadmin: number;
}

export interface PortOccupant {
  port: number;
  pid?: number | null;
  processName?: string | null;
  hint?: string | null;
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
