import { useCallback, useEffect, useState } from "react";
import * as client from "../client";
import type { InstalledPackage, PackageVersion, ProgressEvent } from "../types";

/** Survives tab unmount/remount so "Loading versions…" never flashes after first load. */
type VersionCacheEntry = {
  versions: PackageVersion[];
  installed: InstalledPackage[];
  fromCache: boolean;
  warning: string | null;
  loadedAt: number;
};

const memoryCache = new Map<string, VersionCacheEntry>();

export function peekPackageVersionsCache(packageId: string): VersionCacheEntry | null {
  return memoryCache.get(packageId) ?? null;
}

export function usePackageVersions(packageId: string) {
  const cached = memoryCache.get(packageId);
  const [versions, setVersions] = useState<PackageVersion[]>(
    () => cached?.versions ?? [],
  );
  const [installed, setInstalled] = useState<InstalledPackage[]>(
    () => cached?.installed ?? [],
  );
  // Only show full-page loading when we have nothing to display yet.
  const [loading, setLoading] = useState(() => !cached);
  const [refreshing, setRefreshing] = useState(false);
  const [fromCache, setFromCache] = useState(() => cached?.fromCache ?? false);
  const [error, setError] = useState<string | null>(
    () => cached?.warning ?? null,
  );
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

  const refresh = useCallback(
    async (opts?: { background?: boolean }) => {
      const background = opts?.background ?? memoryCache.has(packageId);
      if (background) {
        setRefreshing(true);
      } else {
        setLoading(true);
      }
      try {
        const [v, i] = await Promise.all([
          client.listPackageVersions(packageId),
          client.listInstalled(packageId),
        ]);
        applyVersions(v.versions, v.warning);
        setFromCache(v.fromCache);
        setRefreshing(v.refreshing);
        setInstalled(i);
        memoryCache.set(packageId, {
          versions: v.versions,
          installed: i,
          fromCache: v.fromCache,
          warning: v.warning ?? null,
          loadedAt: Date.now(),
        });
      } catch (e) {
        // Keep prior cache on failure — never flash empty.
        if (!memoryCache.has(packageId)) {
          setVersions([]);
        }
        setError(e instanceof Error ? e.message : String(e));
        setRefreshing(false);
      } finally {
        setLoading(false);
      }
    },
    [packageId, applyVersions],
  );

  useEffect(() => {
    // Hydrate from memory instantly; refresh in background when cached.
    void refresh({ background: memoryCache.has(packageId) });
  }, [refresh, packageId]);

  useEffect(() => {
    let unlistenProgress: () => void = () => {};
    let unlistenVersions: () => void = () => {};
    void client
      .onInstallProgress((ev) => {
        if (ev.packageId === packageId) setProgress(ev);
      })
      .then((fn) => {
        unlistenProgress = fn;
      });
    void client
      .onVersionsUpdated((ev) => {
        if (ev.packageId !== packageId) return;
        if (ev.error) {
          setError(ev.error);
          setRefreshing(false);
          return;
        }
        applyVersions(ev.versions, null);
        setFromCache(false);
        setRefreshing(false);
        const prev = memoryCache.get(packageId);
        memoryCache.set(packageId, {
          versions: ev.versions,
          installed: prev?.installed ?? [],
          fromCache: false,
          warning: null,
          loadedAt: Date.now(),
        });
      })
      .then((fn) => {
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
      await refresh({ background: true });
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
      await refresh({ background: true });
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
