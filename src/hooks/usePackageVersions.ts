import { useCallback, useEffect, useState } from "react";
import * as client from "../client";
import type { InstalledPackage, PackageVersion, ProgressEvent } from "../types";

export function usePackageVersions(packageId: string) {
  const [versions, setVersions] = useState<PackageVersion[]>([]);
  const [installed, setInstalled] = useState<InstalledPackage[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [progress, setProgress] = useState<ProgressEvent | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [v, i] = await Promise.all([
        client.listPackageVersions(packageId),
        client.listInstalled(packageId),
      ]);
      setVersions(v);
      setInstalled(i);
      // Database providers must never succeed with an empty list — treat that
      // as a fetch failure so the UI shows an error instead of a blank table.
      if (
        v.length === 0 &&
        ["mysql", "mariadb", "postgresql", "mongodb"].includes(packageId)
      ) {
        setError(
          `No ${packageId} versions were returned. The version catalog may be unreachable.`,
        );
      }
    } catch (e) {
      setVersions([]);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [packageId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let unlisten: () => void = () => {};
    void client.onInstallProgress((ev) => {
      if (ev.packageId === packageId) setProgress(ev);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten();
    };
  }, [packageId]);

  const install = async (version: string) => {
    setBusyKey(version);
    setError(null);
    try {
      await client.installPackage(packageId, version);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyKey(null);
      setProgress(null);
    }
  };

  const uninstall = async (version: string) => {
    setBusyKey(version);
    setError(null);
    try {
      await client.uninstallPackage(packageId, version);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyKey(null);
    }
  };

  return {
    versions,
    installed,
    loading,
    error,
    busyKey,
    progress,
    install,
    uninstall,
    refresh,
  };
}
