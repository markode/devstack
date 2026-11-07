import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppInfo,
  BackupResult,
  CreateWordPressRequest,
  CreateWordPressResult,
  DuplicateRequest,
  DuplicateResult,
  InstalledPackage,
  JobProgress,
  PackageInfo,
  PackageVersion,
  PhpExtensionView,
  ProgressEvent,
  SiteDetails,
  SiteRecord,
  StackStatus,
  ManagedServiceStatus,
  TemplateRecord,
} from "./types";

export const isTauri = (): boolean =>
  typeof window !== "undefined" &&
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  Boolean((window as any).__TAURI_INTERNALS__ || (window as any).__TAURI__);

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("get_app_info");
}

export async function listPackages(): Promise<PackageInfo[]> {
  return invoke<PackageInfo[]>("list_packages");
}

export async function listPackageVersions(
  packageId: string,
): Promise<PackageVersion[]> {
  return invoke<PackageVersion[]>("list_package_versions", { packageId });
}

export async function listInstalled(
  packageId?: string,
): Promise<InstalledPackage[]> {
  return invoke<InstalledPackage[]>("list_installed", {
    packageId: packageId ?? null,
  });
}

export async function installPackage(
  packageId: string,
  version: string,
): Promise<InstalledPackage> {
  return invoke<InstalledPackage>("install_package", { packageId, version });
}

export async function uninstallPackage(
  packageId: string,
  version: string,
): Promise<void> {
  return invoke("uninstall_package", { packageId, version });
}

export async function getSites(): Promise<SiteRecord[]> {
  return invoke<SiteRecord[]>("get_sites");
}

export async function listSiteDetails(): Promise<SiteDetails[]> {
  return invoke<SiteDetails[]>("list_site_details");
}

export async function getSiteDetails(siteId: string): Promise<SiteDetails> {
  return invoke<SiteDetails>("get_site_details", { siteId });
}

export async function createSite(
  name: string,
  path: string,
  phpVersion?: string | null,
): Promise<SiteRecord> {
  return invoke<SiteRecord>("create_site_cmd", {
    name,
    path,
    phpVersion: phpVersion ?? null,
  });
}

export async function updateSiteEnvironment(
  siteId: string,
  patch: {
    phpVersion?: string | null;
    nodeVersion?: string | null;
    https?: boolean;
    webServer?: string | null;
  },
): Promise<SiteRecord> {
  return invoke<SiteRecord>("update_site_environment", {
    siteId,
    phpVersion: patch.phpVersion ?? null,
    nodeVersion: patch.nodeVersion ?? null,
    https: patch.https ?? null,
    webServer: patch.webServer ?? null,
  });
}

export async function listPhpExtensionSettings(): Promise<PhpExtensionView[]> {
  return invoke<PhpExtensionView[]>("list_php_extension_settings");
}

export async function setPhpXdebug(
  version: string,
  enabled: boolean,
): Promise<PhpExtensionView> {
  return invoke<PhpExtensionView>("set_php_xdebug", { version, enabled });
}

export async function setPhpManifestUrl(url: string | null): Promise<void> {
  return invoke("set_php_manifest_url", { url });
}

export async function getStackStatus(): Promise<StackStatus> {
  return invoke<StackStatus>("get_stack_status");
}

export async function startDevStack(): Promise<StackStatus> {
  return invoke<StackStatus>("start_dev_stack");
}

export async function stopDevStack(): Promise<StackStatus> {
  return invoke<StackStatus>("stop_dev_stack");
}

export async function applyDevStack(): Promise<StackStatus> {
  return invoke<StackStatus>("apply_dev_stack");
}

export async function onInstallProgress(
  handler: (ev: ProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProgressEvent>("install-progress", (e) => handler(e.payload));
}

export async function onJobProgress(
  handler: (ev: JobProgress) => void,
): Promise<UnlistenFn> {
  return listen<JobProgress>("job-progress", (e) => handler(e.payload));
}

export async function createWordpress(
  request: CreateWordPressRequest,
): Promise<CreateWordPressResult> {
  return invoke<CreateWordPressResult>("create_wordpress", { request });
}

export async function markTemplate(siteId: string): Promise<TemplateRecord> {
  return invoke<TemplateRecord>("mark_template", { siteId });
}

export async function getTemplates(): Promise<TemplateRecord[]> {
  return invoke<TemplateRecord[]>("get_templates");
}

export async function duplicateSite(
  request: DuplicateRequest,
): Promise<DuplicateResult> {
  return invoke<DuplicateResult>("duplicate_site", { request });
}

export async function backupSiteDatabase(
  siteId: string,
): Promise<BackupResult> {
  return invoke<BackupResult>("backup_site_database", { siteId });
}

export async function restoreSiteDatabase(
  siteId: string,
  archivePath: string,
): Promise<void> {
  return invoke("restore_site_database", { siteId, archivePath });
}

export async function listManagedServices(): Promise<ManagedServiceStatus[]> {
  return invoke<ManagedServiceStatus[]>("list_managed_services_cmd");
}

export async function startManagedService(
  serviceId: string,
): Promise<ManagedServiceStatus> {
  return invoke<ManagedServiceStatus>("start_managed_service", { serviceId });
}

export async function stopManagedService(
  serviceId: string,
): Promise<ManagedServiceStatus> {
  return invoke<ManagedServiceStatus>("stop_managed_service", { serviceId });
}

export async function getServiceLog(
  serviceId: string,
  lines?: number,
): Promise<string> {
  return invoke<string>("get_service_log", {
    serviceId,
    lines: lines ?? 80,
  });
}

export async function updatePackage(
  packageId: string,
): Promise<InstalledPackage> {
  return invoke<InstalledPackage>("update_package", { packageId });
}
