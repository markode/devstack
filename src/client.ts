import * as api from "./api";
import * as mock from "./mock";
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
): Promise<PackageVersion[]> {
  if (useMock()) return mock.mockListVersions(packageId);
  return api.listPackageVersions(packageId);
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

export async function getSiteDetails(siteId: string): Promise<SiteDetails> {
  if (useMock()) return mock.mockGetSiteDetails(siteId);
  return api.getSiteDetails(siteId);
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

export { useMock };
