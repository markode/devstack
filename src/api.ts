import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
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
  SitesSettingsPatch,
  DumpStatus,
  ToolsStatus,
  XdebugLaunchHints,
  CreateWordPressRequest,
  CreateWordPressResult,
  DeveloperSettings,
  DuplicateRequest,
  DuplicateResult,
  InstalledPackage,
  JobProgress,
  HostsStatus,
  LogEntry,
  LogLevel,
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

const SILENT = new Set(["append_app_log", "get_app_logs", "get_app_log_error_count"]);

/** Log every Tauri command call + full args/result/error (dev default: no redaction). */
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const silent = SILENT.has(cmd);
  if (!silent) {
    try {
      await tauriInvoke("append_app_log", {
        level: "info",
        source: "command",
        message: `→ ${cmd} ${JSON.stringify(args ?? {})}`,
      });
    } catch {
      /* logger not ready */
    }
  }
  try {
    const result = await tauriInvoke<T>(cmd, args);
    if (!silent) {
      try {
        const summary =
          result === undefined || result === null
            ? "null"
            : typeof result === "object"
              ? JSON.stringify(result).slice(0, 2000)
              : String(result).slice(0, 2000);
        await tauriInvoke("append_app_log", {
          level: "info",
          source: "command",
          message: `← ${cmd} ok ${summary}`,
        });
      } catch {
        /* ignore */
      }
    }
    return result;
  } catch (e) {
    if (!silent) {
      try {
        await tauriInvoke("append_app_log", {
          level: "error",
          source: "command",
          message: `← ${cmd} error ${e instanceof Error ? e.stack ?? e.message : String(e)}`,
        });
      } catch {
        /* ignore */
      }
    }
    throw e;
  }
}

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

/** Fast list — local config only, no TCP/HTTP probes. */
export async function listSiteDetails(): Promise<SiteDetails[]> {
  return invoke<SiteDetails[]>("list_site_details");
}

/** Background reachability probes (TCP+HTTP). Use after fast list. */
export async function probeAllSiteReachability(): Promise<SiteDetails[]> {
  return invoke<SiteDetails[]>("probe_all_site_reachability");
}

export async function listSiteDetailsProbed(): Promise<SiteDetails[]> {
  return invoke<SiteDetails[]>("list_site_details_probed");
}

export async function getSiteDetails(siteId: string): Promise<SiteDetails> {
  return invoke<SiteDetails>("get_site_details", { siteId });
}

export async function repairSiteUrl(siteId: string): Promise<string> {
  return invoke<string>("repair_site_url_cmd", { siteId });
}

export async function createSite(
  name: string,
  path: string,
  phpVersion?: string | null,
  opts?: {
    webServer?: string | null;
    https?: boolean | null;
    documentRoot?: string | null;
  },
): Promise<SiteRecord> {
  return invoke<SiteRecord>("create_site_cmd", {
    name,
    path,
    phpVersion: phpVersion ?? null,
    webServer: opts?.webServer ?? null,
    https: opts?.https ?? null,
    documentRoot: opts?.documentRoot ?? null,
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

export async function mintPhpmyadminSignonUrl(
  siteId: string,
): Promise<string> {
  return invoke<string>("mint_phpmyadmin_signon_url", { siteId });
}

export async function getAppLogs(limit?: number): Promise<LogEntry[]> {
  return invoke<LogEntry[]>("get_app_logs", { limit: limit ?? null });
}

export async function getAppLogErrorCount(): Promise<number> {
  return invoke<number>("get_app_log_error_count");
}

export async function clearAppLogs(): Promise<void> {
  return invoke("clear_app_logs");
}

export async function clearAppLogErrorCount(): Promise<void> {
  return invoke("clear_app_log_error_count");
}

export async function openAppLogFolder(): Promise<string> {
  return invoke<string>("open_app_log_folder");
}

export async function appendAppLog(
  level: LogLevel,
  source: string,
  message: string,
): Promise<void> {
  return tauriInvoke("append_app_log", { level, source, message });
}

export async function onAppLog(
  handler: (entry: LogEntry) => void,
): Promise<UnlistenFn> {
  return listen<LogEntry>("app-log", (ev) => handler(ev.payload));
}

export async function getDeveloperSettings(): Promise<DeveloperSettings> {
  return invoke<DeveloperSettings>("get_developer_settings");
}

export async function updateDeveloperSettings(patch: {
  showLogsTab?: boolean;
  redactSecrets?: boolean;
}): Promise<DeveloperSettings> {
  return invoke<DeveloperSettings>("update_developer_settings", {
    showLogsTab: patch.showLogsTab ?? null,
    redactSecrets: patch.redactSecrets ?? null,
  });
}

/** Tauri reports unregistered commands as `Command <name> not found`. */
export function isMissingCommand(e: unknown): boolean {
  const msg = e instanceof Error ? e.message : String(e);
  return /command .* not found|unknown command/i.test(msg);
}

/** Try each command name in order; later names cover backend renames. */
async function invokeAny<T>(
  cmds: string[],
  args?: Record<string, unknown>,
): Promise<T> {
  let last: unknown;
  for (const cmd of cmds) {
    try {
      return await invoke<T>(cmd, args);
    } catch (e) {
      if (!isMissingCommand(e)) throw e;
      last = e;
    }
  }
  throw last;
}

export async function getSitesSettings(): Promise<SitesSettings> {
  return invoke<SitesSettings>("get_sites_settings");
}

export async function updateSitesSettings(
  patch: SitesSettingsPatch,
): Promise<SitesSettings> {
  return invoke<SitesSettings>("update_sites_settings", {
    parkedDirs: patch.parkedDirs ?? null,
    watchEnabled: patch.watchEnabled ?? null,
    optInPath: patch.optInPath ?? null,
  });
}

export async function syncParkedSites(): Promise<ParkedSyncResult> {
  return invokeAny<ParkedSyncResult>(["sync_parked_sites_cmd", "sync_parked_sites"]);
}

export async function startSiteShare(siteId: string): Promise<ShareStatus> {
  return invoke<ShareStatus>("start_site_share", { siteId });
}

export async function stopSiteShare(): Promise<ShareStatus> {
  return invoke<ShareStatus>("stop_site_share");
}

export async function getShareStatus(): Promise<ShareStatus> {
  return invoke<ShareStatus>("get_share_status");
}

export async function listPhpDumps(limit?: number): Promise<PhpDump[]> {
  return invoke<PhpDump[]>("list_php_dumps", { limit: limit ?? null });
}

export async function clearPhpDumps(): Promise<void> {
  return invoke("clear_php_dumps");
}

export async function getDumpStatus(): Promise<DumpStatus> {
  return invoke<DumpStatus>("get_dump_status");
}

export async function onPhpDump(
  handler: (dump: PhpDump) => void,
): Promise<UnlistenFn> {
  return listen<PhpDump>("php-dump", (e) => handler(e.payload));
}

export async function listSiteBackups(siteId?: string): Promise<SiteBackup[]> {
  return invokeAny<SiteBackup[]>(["list_site_backups_cmd", "list_site_backups"], {
    siteId: siteId ?? null,
  });
}

export async function createSiteBackup(siteId: string): Promise<SiteBackup> {
  return invokeAny<SiteBackup>(["backup_site_full", "create_site_backup"], {
    siteId,
  });
}

export async function restoreSiteBackup(backupId: string): Promise<SiteBackup> {
  return invokeAny<SiteBackup>(
    ["restore_site_backup_cmd", "restore_site_backup"],
    { backupId },
  );
}

export async function deleteSiteBackup(backupId: string): Promise<void> {
  return invokeAny<void>(["delete_site_backup_cmd", "delete_site_backup"], {
    backupId,
  });
}

export async function getBackupSettings(): Promise<BackupSettings> {
  return invoke<BackupSettings>("get_backup_settings");
}

export async function updateBackupSettings(
  patch: BackupSettingsPatch,
): Promise<BackupSettings> {
  return invoke<BackupSettings>("update_backup_settings", {
    folder: patch.folder ?? null,
    intervalSecs: patch.intervalSecs ?? null,
    retain: patch.retain ?? null,
  });
}

export async function setSiteXdebug(
  siteId: string,
  enabled: boolean,
): Promise<XdebugLaunchHints> {
  return invoke<XdebugLaunchHints>("set_site_xdebug", { siteId, enabled });
}

/**
 * Read-only hints for an already-enabled site. Falls back to re-applying the
 * (idempotent) enable when the backend has no dedicated getter.
 */
export async function getXdebugLaunchHints(
  siteId: string,
): Promise<XdebugLaunchHints> {
  try {
    return await invoke<XdebugLaunchHints>("get_xdebug_launch_hints", { siteId });
  } catch (e) {
    if (!isMissingCommand(e)) throw e;
    return setSiteXdebug(siteId, true);
  }
}

export async function ensureDevTools(): Promise<ToolsStatus> {
  return invokeAny<ToolsStatus>(["install_dev_tools", "ensure_dev_tools"]);
}

export async function getToolsStatus(): Promise<ToolsStatus> {
  return invoke<ToolsStatus>("get_tools_status");
}

export async function openSiteTerminal(siteId: string): Promise<void> {
  return invoke("open_site_terminal", { siteId });
}

export async function cloneSite(
  request: DuplicateRequest,
): Promise<DuplicateResult> {
  try {
    return await invoke<DuplicateResult>("clone_site", { request });
  } catch (e) {
    if (isMissingCommand(e)) return duplicateSite(request);
    throw e;
  }
}
