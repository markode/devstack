import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppInfo,
  InstalledPackage,
  PackageInfo,
  PackageVersion,
  PhpExtensionView,
  ProgressEvent,
  SiteRecord,
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

export async function updateSiteEnvironment(
  siteId: string,
  patch: {
    phpVersion?: string | null;
    nodeVersion?: string | null;
    https?: boolean;
  },
): Promise<SiteRecord> {
  return invoke<SiteRecord>("update_site_environment", {
    siteId,
    phpVersion: patch.phpVersion ?? null,
    nodeVersion: patch.nodeVersion ?? null,
    https: patch.https ?? null,
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

export async function onInstallProgress(
  handler: (ev: ProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProgressEvent>("install-progress", (e) => handler(e.payload));
}
