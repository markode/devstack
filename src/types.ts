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

export interface AppInfo {
  name: string;
  version: string;
  dataDir: string;
  os: string;
  arch: string;
  phpManifestUrl?: string | null;
}

export interface SiteRecord {
  id: string;
  name: string;
  path: string;
  phpVersion?: string | null;
  nodeVersion?: string | null;
  https: boolean;
}

export interface PhpExtensionView {
  version: string;
  xdebug: boolean;
  iniPath?: string | null;
}
