import * as api from "./api";
import * as mock from "./mock";
import type {
  AppInfo,
  InstalledPackage,
  PackageInfo,
  PackageVersion,
  PhpExtensionView,
  ProgressEvent,
  SiteDetails,
  SiteRecord,
  StackStatus,
} from "./types";

const useMock = () => !api.isTauri();

export async function getAppInfo(): Promise<AppInfo> {
  if (useMock()) return mock.mockAppInfo;
  return api.getAppInfo();
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
  },
): Promise<SiteRecord> {
  if (useMock()) {
    return mock.mockUpdateSite(siteId, {
      phpVersion: patch.phpVersion,
      nodeVersion: patch.nodeVersion,
      https: patch.https,
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
  if (useMock()) return { ...mock.mockStackStatus, nginx: true, database: true };
  return api.startDevStack();
}

export async function stopDevStack(): Promise<StackStatus> {
  if (useMock()) return { ...mock.mockStackStatus, nginx: false, database: false };
  return api.stopDevStack();
}

export async function onInstallProgress(
  handler: (ev: ProgressEvent) => void,
): Promise<() => void> {
  if (useMock()) return () => undefined;
  return api.onInstallProgress(handler);
}

export { useMock };
