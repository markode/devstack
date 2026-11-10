import * as api from "./api";
import * as mock from "./mock";
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
  DuplicateRequest,
  DuplicateResult,
  HostsStatus,
  InstalledPackage,
  JobProgress,
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
import type {
  ConfigArchiveMeta,
  ConfigExportSummary,
  ConfigImportOptions,
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
import type { UnlistenFn } from "@tauri-apps/api/event";

const useMock = () => !api.isTauri();

export async function getAppInfo(): Promise<AppInfo> {
  if (useMock()) return mock.mockAppInfo;
  return api.getAppInfo();
}

export async function getBackupsDir(): Promise<string> {
  if (useMock()) return "/tmp/DevStack/backups";
  return api.getBackupsDir();
}

export async function listPackages(): Promise<PackageInfo[]> {
  if (useMock()) return mock.mockPackages;
  return api.listPackages();
}

export async function listPackageVersions(
  packageId: string,
): Promise<VersionListResponse> {
  if (useMock()) return mock.mockListVersions(packageId);
  return api.listPackageVersions(packageId);
}

export async function onVersionsUpdated(
  handler: (ev: VersionsUpdatedEvent) => void,
): Promise<UnlistenFn> {
  if (useMock()) return () => {};
  return api.onVersionsUpdated(handler);
}

export async function getHostsStatus(): Promise<HostsStatus> {
  if (useMock()) {
    return {
      ok: true,
      missing: [],
      message: null,
      caTrusted: true,
      caMessage: null,
    };
  }
  return api.getHostsStatus();
}

export async function repairHosts(): Promise<HostsStatus> {
  if (useMock()) {
    return {
      ok: true,
      missing: [],
      message: null,
      caTrusted: true,
      caMessage: null,
    };
  }
  return api.repairHosts();
}

export async function getPerformanceSettings(): Promise<
  import("./types").PerformanceSettings
> {
  if (useMock()) {
    return {
      defenderExclusions: false,
      defenderExclusionsSupported: false,
      exclusionPaths: [],
      defenderExclusionsHelp:
        "Windows Defender exclusions are opt-in and require UAC (default off).",
    };
  }
  return api.getPerformanceSettings();
}

export async function setDefenderExclusions(
  enabled: boolean,
): Promise<import("./types").PerformanceSettings> {
  if (useMock()) {
    return {
      defenderExclusions: enabled,
      defenderExclusionsSupported: false,
      exclusionPaths: [],
      defenderExclusionsHelp:
        "Windows Defender exclusions are opt-in and require UAC (default off).",
    };
  }
  return api.setDefenderExclusions(enabled);
}

export async function listInstalled(
  packageId?: string,
): Promise<InstalledPackage[]> {
  if (useMock()) return mock.mockListInstalled(packageId);
  return api.listInstalled(packageId);
}

export async function installPackage(
  packageId: string,
  version: string,
): Promise<InstalledPackage> {
  if (useMock()) return mock.mockInstall(packageId, version);
  return api.installPackage(packageId, version);
}

export async function uninstallPackage(
  packageId: string,
  version: string,
): Promise<void> {
  if (useMock()) return mock.mockUninstall(packageId, version);
  return api.uninstallPackage(packageId, version);
}

export async function getSites(): Promise<SiteRecord[]> {
  if (useMock()) return mock.mockGetSites();
  return api.getSites();
}

export async function listSiteDetails(): Promise<SiteDetails[]> {
  if (useMock()) return mock.mockListSiteDetails();
  return api.listSiteDetails();
}

export async function probeAllSiteReachability(): Promise<SiteDetails[]> {
  if (useMock()) return mock.mockListSiteDetailsProbed();
  return api.probeAllSiteReachability();
}

export async function listSiteDetailsProbed(): Promise<SiteDetails[]> {
  if (useMock()) return mock.mockListSiteDetailsProbed();
  return api.listSiteDetailsProbed();
}

export async function getSiteDetails(siteId: string): Promise<SiteDetails> {
  if (useMock()) return mock.mockGetSiteDetails(siteId);
  return api.getSiteDetails(siteId);
}

export async function repairSiteUrl(siteId: string): Promise<string> {
  if (useMock()) return `mock: repaired ${siteId}`;
  return api.repairSiteUrl(siteId);
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
  if (useMock()) {
    return mock.mockUpdateSite(siteId, {
      phpVersion: patch.phpVersion,
      nodeVersion: patch.nodeVersion,
      https: patch.https,
      webServer: patch.webServer,
    });
  }
  return api.updateSiteEnvironment(siteId, patch);
}

export async function listPhpExtensionSettings(): Promise<PhpExtensionView[]> {
  if (useMock()) return mock.mockPhpExt();
  return api.listPhpExtensionSettings();
}

export async function setPhpXdebug(
  version: string,
  enabled: boolean,
): Promise<PhpExtensionView> {
  if (useMock()) return mock.mockSetXdebug(version, enabled);
  return api.setPhpXdebug(version, enabled);
}

export async function getStackStatus(): Promise<StackStatus> {
  if (useMock()) return mock.mockStackStatus;
  return api.getStackStatus();
}

export async function startDevStack(): Promise<StackStatus> {
  if (useMock())
    return {
      ...mock.mockStackStatus,
      nginx: true,
      apache: true,
      database: true,
    };
  return api.startDevStack();
}

export async function stopDevStack(): Promise<StackStatus> {
  if (useMock())
    return {
      ...mock.mockStackStatus,
      nginx: false,
      apache: false,
      database: false,
    };
  return api.stopDevStack();
}

export async function onInstallProgress(
  handler: (ev: ProgressEvent) => void,
): Promise<() => void> {
  if (useMock()) return () => undefined;
  return api.onInstallProgress(handler);
}

export async function onJobProgress(
  handler: (ev: JobProgress) => void,
): Promise<() => void> {
  if (useMock()) return () => undefined;
  return api.onJobProgress(handler);
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
) {
  if (useMock()) {
    return {
      id: name.toLowerCase().replace(/\s+/g, "-"),
      name,
      path,
      domain: `${name.toLowerCase().replace(/\s+/g, "-")}.test`,
      phpVersion: phpVersion ?? "8.3",
      https: opts?.https ?? true,
      webServer: opts?.webServer ?? "nginx",
      status: "stopped",
    };
  }
  return api.createSite(name, path, phpVersion, opts);
}

export async function createWordpress(
  request: CreateWordPressRequest,
): Promise<CreateWordPressResult> {
  if (useMock()) return mock.mockCreateWordpress(request);
  return api.createWordpress(request);
}

export async function markTemplate(siteId: string): Promise<TemplateRecord> {
  if (useMock()) return mock.mockMarkTemplate(siteId);
  return api.markTemplate(siteId);
}

export async function getTemplates(): Promise<TemplateRecord[]> {
  if (useMock()) return mock.mockGetTemplates();
  return api.getTemplates();
}

export async function duplicateSite(
  request: DuplicateRequest,
): Promise<DuplicateResult> {
  if (useMock()) return mock.mockDuplicateSite(request);
  return api.duplicateSite(request);
}

export async function backupSiteDatabase(
  siteId: string,
): Promise<BackupResult> {
  if (useMock()) return mock.mockBackupSite(siteId);
  return api.backupSiteDatabase(siteId);
}

export async function restoreSiteDatabase(
  siteId: string,
  archivePath: string,
): Promise<void> {
  if (useMock()) return mock.mockRestoreSite(siteId, archivePath);
  return api.restoreSiteDatabase(siteId, archivePath);
}

export async function listManagedServices(): Promise<ManagedServiceStatus[]> {
  if (useMock()) return mock.mockManagedServices();
  return api.listManagedServices();
}

export async function startManagedService(
  serviceId: string,
): Promise<ManagedServiceStatus> {
  if (useMock()) return mock.mockStartService(serviceId);
  return api.startManagedService(serviceId);
}

export async function stopManagedService(
  serviceId: string,
): Promise<ManagedServiceStatus> {
  if (useMock()) return mock.mockStopService(serviceId);
  return api.stopManagedService(serviceId);
}

export async function getServiceLog(
  serviceId: string,
  lines?: number,
): Promise<string> {
  if (useMock()) return mock.mockServiceLog(serviceId);
  return api.getServiceLog(serviceId, lines);
}

export async function updatePackage(
  packageId: string,
): Promise<InstalledPackage> {
  if (useMock()) return mock.mockInstall(packageId, "latest");
  return api.updatePackage(packageId);
}

export async function getDbFilesSettings() {
  if (useMock()) {
    return {
      folder: null,
      autoExportIntervalSecs: 300,
      autoExportOnStop: true,
      importMode: "ask",
    };
  }
  return api.getDbFilesSettings();
}

export async function getDatabasesFolder() {
  if (useMock()) return "/Users/dev/Library/Application Support/DevStack/Databases";
  return api.getDatabasesFolder();
}

export async function updateDbFilesSettings(patch: {
  folder?: string | null;
  autoExportIntervalSecs?: number;
  autoExportOnStop?: boolean;
  importMode?: string;
}) {
  if (useMock()) {
    return {
      folder: patch.folder ?? null,
      autoExportIntervalSecs: patch.autoExportIntervalSecs ?? 300,
      autoExportOnStop: patch.autoExportOnStop ?? true,
      importMode: patch.importMode ?? "ask",
    };
  }
  return api.updateDbFilesSettings(patch);
}

export async function listDatabaseFiles() {
  if (useMock()) return [];
  return api.listDatabaseFiles();
}

export async function exportAllDatabasesToFolder() {
  if (useMock()) return [];
  return api.exportAllDatabasesToFolder();
}

export async function downloadSiteDatabase(siteId: string, destPath: string) {
  if (useMock()) return { path: destPath, bytes: 1_200_000 };
  return api.downloadSiteDatabase(siteId, destPath);
}

export async function importSiteDatabaseFile(
  siteId: string,
  archivePath: string,
  search?: string | null,
  replace?: string | null,
) {
  if (useMock()) return;
  return api.importSiteDatabaseFile(siteId, archivePath, search, replace);
}

export async function getPortsConfig() {
  if (useMock()) return mock.mockPortsConfig();
  return api.getPortsConfig();
}

export async function updatePortsConfig(
  config: import("./types").PortsConfig,
) {
  if (useMock()) return mock.mockUpdatePortsConfig(config);
  return api.updatePortsConfig(config);
}

export async function setCoexistenceMode(enabled: boolean) {
  if (useMock()) return mock.mockSetCoexistenceMode(enabled);
  return api.setCoexistenceMode(enabled);
}

export async function checkServicePorts(serviceId: string) {
  if (useMock()) return mock.mockCheckServicePorts(serviceId);
  return api.checkServicePorts(serviceId);
}

export async function applyNextFreePorts(serviceId: string) {
  if (useMock()) return mock.mockApplyNextFreePorts(serviceId);
  return api.applyNextFreePorts(serviceId);
}

export async function mintPhpmyadminSignonUrl(siteId: string): Promise<string> {
  if (useMock()) {
    return `https://phpmyadmin.test/devstack-signon.php?token=mocktoken`;
  }
  return api.mintPhpmyadminSignonUrl(siteId);
}

export { useMock };

export async function getAppLogs(limit?: number) {
  if (useMock()) return mock.mockGetAppLogs(limit);
  return api.getAppLogs(limit);
}

export async function getAppLogErrorCount() {
  if (useMock()) return mock.mockLogState.errorCount;
  return api.getAppLogErrorCount();
}

export async function clearAppLogs() {
  if (useMock()) return mock.mockClearAppLogs();
  return api.clearAppLogs();
}

export async function clearAppLogErrorCount() {
  if (useMock()) {
    mock.mockLogState.errorCount = 0;
    return;
  }
  return api.clearAppLogErrorCount();
}

export async function openAppLogFolder() {
  if (useMock()) return "/tmp/DevStack/logs";
  return api.openAppLogFolder();
}

export async function appendAppLog(
  level: import("./types").LogLevel,
  source: string,
  message: string,
) {
  if (useMock()) return mock.mockAppendAppLog(level, source, message);
  return api.appendAppLog(level, source, message);
}

export async function onAppLog(
  handler: (entry: import("./types").LogEntry) => void,
) {
  if (useMock()) return () => {};
  return api.onAppLog(handler);
}

export async function getDeveloperSettings() {
  if (useMock()) return { ...mock.mockDeveloperSettingsState };
  return api.getDeveloperSettings();
}

export async function updateDeveloperSettings(patch: {
  showLogsTab?: boolean;
  redactSecrets?: boolean;
}) {
  if (useMock()) {
    Object.assign(mock.mockDeveloperSettingsState, patch);
    return { ...mock.mockDeveloperSettingsState };
  }
  return api.updateDeveloperSettings(patch);
}

export async function getSitesSettings(): Promise<SitesSettings> {
  if (useMock()) return mock.mockGetSitesSettings();
  return api.getSitesSettings();
}

export async function updateSitesSettings(
  patch: SitesSettingsPatch,
): Promise<SitesSettings> {
  if (useMock()) return mock.mockUpdateSitesSettings(patch);
  return api.updateSitesSettings(patch);
}

export async function syncParkedSites(): Promise<ParkedSyncResult> {
  if (useMock()) return mock.mockSyncParkedSites();
  return api.syncParkedSites();
}

export async function startSiteShare(siteId: string): Promise<ShareStatus> {
  if (useMock()) return mock.mockStartShare(siteId);
  return api.startSiteShare(siteId);
}

export async function stopSiteShare(): Promise<ShareStatus> {
  if (useMock()) return mock.mockStopShare();
  return api.stopSiteShare();
}

export async function getShareStatus(): Promise<ShareStatus> {
  if (useMock()) return mock.mockGetShareStatus();
  return api.getShareStatus();
}

export async function listPhpDumps(limit?: number): Promise<PhpDump[]> {
  if (useMock()) return mock.mockListDumps(limit);
  return api.listPhpDumps(limit);
}

export async function clearPhpDumps(): Promise<void> {
  if (useMock()) return mock.mockClearDumps();
  return api.clearPhpDumps();
}

export async function getDumpStatus(): Promise<DumpStatus> {
  if (useMock()) return { port: 9912, helperPath: "/tmp/DevStack/devstack_dump.php" };
  return api.getDumpStatus();
}

export async function onPhpDump(
  handler: (dump: PhpDump) => void,
): Promise<() => void> {
  if (useMock()) return () => undefined;
  return api.onPhpDump(handler);
}

export async function listSiteBackups(siteId?: string): Promise<SiteBackup[]> {
  if (useMock()) return mock.mockListBackups(siteId);
  return api.listSiteBackups(siteId);
}

export async function createSiteBackup(siteId: string): Promise<SiteBackup> {
  if (useMock()) return mock.mockCreateBackup(siteId);
  return api.createSiteBackup(siteId);
}

export async function restoreSiteBackup(backupId: string): Promise<SiteBackup> {
  if (useMock()) return mock.mockRestoreBackup(backupId);
  return api.restoreSiteBackup(backupId);
}

export async function deleteSiteBackup(backupId: string): Promise<void> {
  if (useMock()) return mock.mockDeleteBackup(backupId);
  return api.deleteSiteBackup(backupId);
}

export async function getBackupSettings(): Promise<BackupSettings> {
  if (useMock()) return mock.mockGetBackupSettings();
  return api.getBackupSettings();
}

export async function updateBackupSettings(
  patch: BackupSettingsPatch,
): Promise<BackupSettings> {
  if (useMock()) return mock.mockUpdateBackupSettings(patch);
  return api.updateBackupSettings(patch);
}

export async function setSiteXdebug(
  siteId: string,
  enabled: boolean,
): Promise<XdebugLaunchHints> {
  if (useMock()) return mock.mockSetSiteXdebug(siteId, enabled);
  return api.setSiteXdebug(siteId, enabled);
}

export async function getXdebugLaunchHints(
  siteId: string,
): Promise<XdebugLaunchHints> {
  if (useMock()) return mock.mockXdebugHints(siteId);
  return api.getXdebugLaunchHints(siteId);
}

export async function ensureDevTools(): Promise<ToolsStatus> {
  if (useMock()) return mock.mockToolsStatus();
  return api.ensureDevTools();
}

export async function getToolsStatus(): Promise<ToolsStatus> {
  if (useMock()) return mock.mockToolsStatus();
  return api.getToolsStatus();
}

export async function openSiteTerminal(siteId: string): Promise<void> {
  if (useMock()) return;
  return api.openSiteTerminal(siteId);
}

/** Clone a site or template (backend `clone_site`, falls back to `duplicate_site`). */
export async function cloneSite(
  request: DuplicateRequest,
): Promise<DuplicateResult> {
  if (useMock()) return mock.mockDuplicateSite(request);
  return api.cloneSite(request);
}

// --- v0.3.0 ---------------------------------------------------------------------

export async function setReverbSite(siteId: string | null): Promise<ManagedServiceStatus> {
  if (useMock()) return mock.mockSetReverbSite(siteId);
  return api.setReverbSite(siteId);
}

export async function getSitePhpIni(siteId: string): Promise<SitePhpIniView> {
  if (useMock()) return mock.mockGetSitePhpIni(siteId);
  return api.getSitePhpIni(siteId);
}

export async function setSitePhpIni(
  siteId: string,
  overrides: Record<string, string>,
): Promise<SitePhpIniView> {
  if (useMock()) return mock.mockSetSitePhpIni(siteId, overrides);
  return api.setSitePhpIni(siteId, overrides);
}

export async function setSitePhpIniSliders(
  siteId: string,
  sliders: PhpIniSliders,
): Promise<SitePhpIniView> {
  if (useMock()) return mock.mockSetSitePhpIniSliders(siteId, sliders);
  return api.setSitePhpIniSliders(siteId, sliders);
}

export async function listSiteTemplateKinds(): Promise<SiteTemplateInfo[]> {
  if (useMock()) return mock.mockSiteTemplateKinds();
  return api.listSiteTemplateKinds();
}

export async function createSiteFromTemplate(
  request: CreateFromTemplateRequest,
): Promise<CreateFromTemplateResult> {
  if (useMock()) return mock.mockCreateFromTemplate(request);
  return api.createSiteFromTemplate(request);
}

export async function hostingToolsStatus(): Promise<HostingTools> {
  if (useMock()) return mock.mockHostingTools();
  return api.hostingToolsStatus();
}

export async function hostingListProfiles(): Promise<HostingProfile[]> {
  if (useMock()) return mock.mockHostingProfiles();
  return api.hostingListProfiles();
}

export async function hostingSaveProfile(
  profile: HostingProfile,
  password?: string | null,
  dbPassword?: string | null,
): Promise<HostingProfile> {
  if (useMock()) return mock.mockHostingSave(profile, password);
  return api.hostingSaveProfile(profile, password, dbPassword);
}

export async function hostingDeleteProfile(profileId: string): Promise<void> {
  if (useMock()) return mock.mockHostingDelete(profileId);
  return api.hostingDeleteProfile(profileId);
}

export async function hostingTestConnection(profileId: string): Promise<string> {
  if (useMock()) return `Connected (mock) to ${profileId}`;
  return api.hostingTestConnection(profileId);
}

export async function hostingPreview(
  profileId: string,
  direction: SyncDirection,
  del = false,
): Promise<SyncPlan> {
  if (useMock()) return mock.mockHostingPreview(profileId, direction);
  return api.hostingPreview(profileId, direction, del);
}

export async function hostingRunSync(
  profileId: string,
  direction: SyncDirection,
  opts: { delete?: boolean; includeDatabase?: boolean } = {},
): Promise<SyncResult> {
  if (useMock()) return mock.mockHostingRun(profileId, direction, opts.includeDatabase ?? false);
  return api.hostingRunSync(profileId, direction, opts);
}

export async function getTldSettings(): Promise<TldSettings> {
  if (useMock()) return mock.mockTldSettings();
  return api.getTldSettings();
}

export async function setDefaultTld(tld: string): Promise<TldSettings> {
  if (useMock()) return mock.mockSetDefaultTld(tld);
  return api.setDefaultTld(tld);
}

export async function updateSiteDomain(
  siteId: string,
  patch: { tld?: string; aliases?: string[]; wildcard?: boolean },
): Promise<SiteDomainUpdate> {
  if (useMock()) return mock.mockUpdateSiteDomain(siteId, patch);
  return api.updateSiteDomain(siteId, patch);
}

export async function listSiteLogSources(siteId: string): Promise<LogSourceInfo[]> {
  if (useMock()) return mock.mockLogSources(siteId);
  return api.listSiteLogSources(siteId);
}

export async function readSiteLogs(
  siteId: string,
  query: SiteLogQuery,
): Promise<SiteLogPage> {
  if (useMock()) return mock.mockReadSiteLogs(siteId, query);
  return api.readSiteLogs(siteId, query);
}

export async function getSiteQueueStatus(siteId: string): Promise<QueueStatus> {
  if (useMock()) return mock.mockQueueStatus(siteId);
  return api.getSiteQueueStatus(siteId);
}

export async function getMobilePreviewStatus(): Promise<MobilePreviewInfo> {
  if (useMock()) return mock.mockMobilePreview();
  return api.getMobilePreviewStatus();
}

export async function startMobilePreview(siteId: string): Promise<MobilePreviewInfo> {
  if (useMock()) return mock.mockStartMobilePreview(siteId);
  return api.startMobilePreview(siteId);
}

export async function stopMobilePreview(): Promise<MobilePreviewInfo> {
  if (useMock()) return mock.mockStopMobilePreview();
  return api.stopMobilePreview();
}

export async function getMetricsSnapshot(): Promise<MetricsSnapshot> {
  if (useMock()) return mock.mockMetricsSnapshot();
  return api.getMetricsSnapshot();
}

export async function getSlowQueryInfo(limit = 50): Promise<SlowQueryInfo> {
  if (useMock()) return mock.mockSlowQueryInfo();
  return api.getSlowQueryInfo(limit);
}

export async function setSlowQueryLog(
  enabled: boolean,
  thresholdSecs = 1,
): Promise<SlowQueryInfo> {
  if (useMock()) return mock.mockSetSlowQueryLog(enabled, thresholdSecs);
  return api.setSlowQueryLog(enabled, thresholdSecs);
}

export async function getSpxStatus(phpVersion?: string | null): Promise<SpxStatus> {
  if (useMock()) return mock.mockSpxStatus();
  return api.getSpxStatus(phpVersion);
}

export async function setSpxEnabled(
  enabled: boolean,
  phpVersion?: string | null,
): Promise<SpxStatus> {
  if (useMock()) return mock.mockSetSpx(enabled);
  return api.setSpxEnabled(enabled, phpVersion);
}

export async function ensureSpxExtension(
  phpVersion?: string | null,
): Promise<SpxStatus> {
  if (useMock()) return mock.mockEnsureSpx();
  return api.ensureSpxExtension(phpVersion);
}

export async function enableXdebugProfiling(
  phpVersion?: string | null,
): Promise<SpxStatus> {
  if (useMock()) return mock.mockEnableXdebugProfiling();
  return api.enableXdebugProfiling(phpVersion);
}

export async function exportConfig(
  destPath: string,
  includeDatabases = false,
): Promise<ConfigExportSummary> {
  if (useMock()) return mock.mockExportConfig(destPath, includeDatabases);
  return api.exportConfig(destPath, includeDatabases);
}

export async function inspectConfigArchive(path: string): Promise<ConfigArchiveMeta> {
  if (useMock()) return mock.mockInspectConfig(path);
  return api.inspectConfigArchive(path);
}

export async function importConfig(
  path: string,
  options?: ConfigImportOptions,
): Promise<ConfigImportReport> {
  if (useMock()) return mock.mockImportConfig(path);
  return api.importConfig(path, options);
}
