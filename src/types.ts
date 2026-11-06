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
  dns: boolean;
  database: boolean;
  databaseEngine?: string | null;
  httpPort: number;
  httpsPort: number;
  phpVersions: string[];
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

export interface SiteRecord {
  id: string;
  name: string;
  path: string;
  domain: string;
  phpVersion?: string | null;
  nodeVersion?: string | null;
  https: boolean;
  framework?: string | null;
  databaseEngine?: string | null;
  databaseName?: string | null;
  databaseUser?: string | null;
  status?: string;
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
}

export interface PhpExtensionView {
  version: string;
  xdebug: boolean;
  iniPath?: string | null;
}
