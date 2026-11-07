import { useCallback, useEffect, useState } from "react";
import * as client from "../client";
import type { InstalledPackage, PackageVersion, ProgressEvent } from "../types";

export function usePackageVersions(packageId: string) {
  const [versions, setVersions] = useState<PackageVersion[]>([]);
  const [installed, setInstalled] = useState<InstalledPackage[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [fromCache, setFromCache] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [progress, setProgress] = useState<ProgressEvent | null>(null);

  const applyVersions = useCallback(
    (list: PackageVersion[], warning?: string | null) => {
      setVersions(list);
      if (
        list.length === 0 &&
        ["mysql", "mariadb", "postgresql", "mongodb"].includes(packageId)
      ) {
        setError(
          warning ??
            `No ${packageId} versions were returned. The version catalog may be unreachable.`,
        );
      } else if (warning) {
        setError(warning);
      } else {
        setError(null);
      }
    },
    [packageId],
  );

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [v, i] = await Promise.all([
        client.listPackageVersions(packageId),
        client.listInstalled(packageId),
      ]);
      applyVersions(v.versions, v.warning);
      setFromCache(v.fromCache);
      setRefreshing(v.refreshing);
      setInstalled(i);
    } catch (e) {
      setVersions([]);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [packageId, applyVersions]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let unlistenProgress: () => void = () => {};
    let unlistenVersions: () => void = () => {};
    void client.onInstallProgress((ev) => {
      if (ev.packageId === packageId) setProgress(ev);
    }).then((fn) => {
      unlistenProgress = fn;
    });
    void client.onVersionsUpdated((ev) => {
      if (ev.packageId !== packageId) return;
      if (ev.error) {
        setError(ev.error);
        setRefreshing(false);
        return;
      }
      applyVersions(ev.versions, null);
      setFromCache(false);
      setRefreshing(false);
    }).then((fn) => {
      unlistenVersions = fn;
    });
    return () => {
      unlistenProgress();
      unlistenVersions();
    };
  }, [packageId, applyVersions]);

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
    refreshing,
    fromCache,
    error,
    busyKey,
    progress,
    install,
    uninstall,
    refresh,
  };
}
