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
  HostsStatus,
  PackageInfo,
  PhpExtensionView,
  ProgressEvent,
  SiteDetails,
  SiteRecord,
  StackStatus,
  ManagedServiceStatus,
  TemplateRecord,
  VersionListResponse,
  VersionsUpdatedEvent,
} from "./types";

export const isTauri = (): boolean =>
  typeof window !== "undefined" &&
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  Boolean((window as any).__TAURI_INTERNALS__ || (window as any).__TAURI__);

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("get_app_info");
}

export async function getBackupsDir(): Promise<string> {
  return invoke<string>("get_backups_dir");
}

export async function listPackages(): Promise<PackageInfo[]> {
  return invoke<PackageInfo[]>("list_packages");
}

export async function listPackageVersions(
  packageId: string,
): Promise<VersionListResponse> {
  return invoke<VersionListResponse>("list_package_versions", { packageId });
}

export async function onVersionsUpdated(
  handler: (ev: VersionsUpdatedEvent) => void,
): Promise<UnlistenFn> {
  return listen<VersionsUpdatedEvent>("versions-updated", (e) =>
    handler(e.payload),
  );
}

export async function getHostsStatus(): Promise<HostsStatus> {
  return invoke<HostsStatus>("get_hosts_status");
}

export async function repairHosts(): Promise<HostsStatus> {
  return invoke<HostsStatus>("repair_hosts");
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

export async function getDbFilesSettings(): Promise<
  import("./types").DatabaseFilesSettings
> {
  return invoke("get_db_files_settings");
}

export async function getDatabasesFolder(): Promise<string> {
  return invoke<string>("get_databases_folder");
}

export async function updateDbFilesSettings(patch: {
  folder?: string | null;
  autoExportIntervalSecs?: number;
  autoExportOnStop?: boolean;
  importMode?: string;
}): Promise<import("./types").DatabaseFilesSettings> {
  return invoke("update_db_files_settings", {
    folder: patch.folder ?? null,
    autoExportIntervalSecs: patch.autoExportIntervalSecs ?? null,
    autoExportOnStop: patch.autoExportOnStop ?? null,
    importMode: patch.importMode ?? null,
  });
}

export async function listDatabaseFiles(): Promise<
  import("./types").DbFileEntry[]
> {
  return invoke("list_database_files");
}

export async function exportAllDatabasesToFolder(): Promise<
  import("./types").DbFileEntry[]
> {
  return invoke("export_all_databases_to_folder");
}

export async function downloadSiteDatabase(
  siteId: string,
  destPath: string,
): Promise<BackupResult> {
  return invoke<BackupResult>("download_site_database", { siteId, destPath });
}

export async function importSiteDatabaseFile(
  siteId: string,
  archivePath: string,
  search?: string | null,
  replace?: string | null,
): Promise<void> {
  return invoke("import_site_database_file", {
    siteId,
    archivePath,
    search: search ?? null,
    replace: replace ?? null,
  });
}

export async function getPortsConfig(): Promise<import("./types").PortsConfig> {
  return invoke("get_ports_config");
}

export async function updatePortsConfig(
  config: import("./types").PortsConfig,
): Promise<import("./types").PortsConfig> {
  return invoke("update_ports_config", { config });
}

export async function setCoexistenceMode(
  enabled: boolean,
): Promise<import("./types").PortsConfig> {
  return invoke("set_coexistence_mode", { enabled });
}

export async function checkServicePorts(
  serviceId: string,
): Promise<import("./types").PortCheckResult> {
  return invoke("check_service_ports_cmd", { serviceId });
}

export async function applyNextFreePorts(
  serviceId: string,
): Promise<import("./types").PortsConfig> {
  return invoke("apply_next_free_ports", { serviceId });
}
