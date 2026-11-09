import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import * as client from "../client";
import type {
  HostsStatus,
  InstalledPackage,
  SiteDetails,
  StackStatus,
  TemplateRecord,
} from "../types";

export type SitesCache = {
  details: SiteDetails[];
  templates: TemplateRecord[];
  phpInstalled: InstalledPackage[];
  nodeInstalled: InstalledPackage[];
  stack: StackStatus | null;
  hosts: HostsStatus | null;
  /** True after the first successful fast list (empty list can still be loaded). */
  loaded: boolean;
  /** True while the fast list request is in flight. */
  refreshing: boolean;
  /** True while background reachability probes are running. */
  probing: boolean;
  /** True while the cache only holds the localStorage snapshot (not yet refreshed). */
  fromSnapshot: boolean;
};

/** Lightweight first-paint snapshot: sites list (no probe payloads) + stack ports. */
export const SITES_SNAPSHOT_KEY = "devstack.sitesSnapshot.v1";

type SitesSnapshot = {
  v: 1;
  savedAt: number;
  details: SiteDetails[];
  stack: StackStatus | null;
};

function snapshotStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/** Drop reachability payloads (large log excerpts) — they are re-probed anyway. */
export function liteDetails(details: SiteDetails[]): SiteDetails[] {
  return details.map((d) => {
    const { reachability: _reachability, ...rest } = d;
    return rest;
  });
}

export function writeSitesSnapshot(
  details: SiteDetails[],
  stack: StackStatus | null,
): void {
  const storage = snapshotStorage();
  if (!storage) return;
  try {
    const snap: SitesSnapshot = {
      v: 1,
      savedAt: Date.now(),
      details: liteDetails(details),
      stack,
    };
    storage.setItem(SITES_SNAPSHOT_KEY, JSON.stringify(snap));
  } catch {
    // Quota / private mode — snapshot is purely an optimisation.
  }
}

export function readSitesSnapshot(): SitesSnapshot | null {
  const storage = snapshotStorage();
  if (!storage) return null;
  try {
    const raw = storage.getItem(SITES_SNAPSHOT_KEY);
    if (!raw) return null;
    const snap = JSON.parse(raw) as Partial<SitesSnapshot>;
    if (snap.v !== 1 || !Array.isArray(snap.details)) return null;
    return {
      v: 1,
      savedAt: Number(snap.savedAt) || 0,
      details: snap.details,
      stack: snap.stack ?? null,
    };
  } catch {
    return null;
  }
}

function hydrateSites(): SitesCache {
  const snap = readSitesSnapshot();
  if (!snap || snap.details.length === 0) return emptySites;
  return {
    ...emptySites,
    details: snap.details,
    stack: snap.stack,
    loaded: true,
    fromSnapshot: true,
  };
}

type AppDataContextValue = {
  sites: SitesCache;
  /** Load fast list immediately; optionally probe in background. Never clears cache on failure. */
  loadSites: (opts?: { force?: boolean; probe?: boolean }) => Promise<void>;
  /** Mount-time guard: no-op when real (non-snapshot) data is cached or a load is in flight. */
  ensureSites: () => Promise<void>;
  /** Run only the background reachability probe against the current cache. */
  probeSites: () => Promise<void>;
  setSitesDetails: (
    next: SiteDetails[] | ((prev: SiteDetails[]) => SiteDetails[]),
  ) => void;
  setStack: (stack: StackStatus | null) => void;
  patchSitesCache: (patch: Partial<SitesCache>) => void;
};

const emptySites: SitesCache = {
  details: [],
  templates: [],
  phpInstalled: [],
  nodeInstalled: [],
  stack: null,
  hosts: null,
  loaded: false,
  refreshing: false,
  probing: false,
  fromSnapshot: false,
};

const AppDataContext = createContext<AppDataContextValue | null>(null);

export function AppDataProvider({ children }: { children: ReactNode }) {
  const [sites, setSites] = useState<SitesCache>(hydrateSites);
  const gen = useRef(0);
  const inflight = useRef<Promise<void> | null>(null);
  const booted = useRef(false);
  const mounted = useRef(true);

  const safeSetSites = useCallback(
    (updater: SitesCache | ((prev: SitesCache) => SitesCache)) => {
      if (!mounted.current) return;
      setSites(updater);
    },
    [],
  );

  const patchSitesCache = useCallback(
    (patch: Partial<SitesCache>) => {
      safeSetSites((prev) => ({ ...prev, ...patch }));
    },
    [safeSetSites],
  );

  const setSitesDetails = useCallback(
    (next: SiteDetails[] | ((prev: SiteDetails[]) => SiteDetails[])) => {
      safeSetSites((prev) => ({
        ...prev,
        details: typeof next === "function" ? next(prev.details) : next,
        loaded: true,
      }));
    },
    [safeSetSites],
  );

  const setStack = useCallback(
    (stack: StackStatus | null) => {
      safeSetSites((prev) => ({ ...prev, stack }));
    },
    [safeSetSites],
  );

  const probeSites = useCallback(async () => {
    const myGen = gen.current;
    safeSetSites((prev) => ({ ...prev, probing: true }));
    try {
      const probed = await client.probeAllSiteReachability();
      if (!mounted.current || myGen !== gen.current) return;
      safeSetSites((prev) => ({
        ...prev,
        details: probed,
        loaded: true,
        probing: false,
      }));
    } catch {
      if (!mounted.current || myGen !== gen.current) return;
      // Keep fast-list cache; just stop the top loader.
      safeSetSites((prev) => ({ ...prev, probing: false }));
    }
  }, [safeSetSites]);

  const loadSites = useCallback(
    async (opts?: { force?: boolean; probe?: boolean }) => {
      const probe = opts?.probe !== false;
      // Deduplicate concurrent loads unless forced.
      if (inflight.current && !opts?.force) {
        return inflight.current;
      }

      const myGen = ++gen.current;
      safeSetSites((prev) => ({
        ...prev,
        refreshing: true,
        // Keep existing details visible — never clear to [].
      }));

      let runPromise!: Promise<void>;
      runPromise = (async () => {
        try {
          // Phase 1 — only what the tiles need to paint (two cheap reads).
          const [details, st] = await Promise.all([
            client.listSiteDetails(),
            client.getStackStatus(),
          ]);
          if (!mounted.current || myGen !== gen.current) return;
          safeSetSites((prev) => ({
            ...prev,
            details,
            stack: st,
            loaded: true,
            fromSnapshot: false,
            refreshing: false,
            probing: probe,
          }));

          // Phase 2 — secondary lists after the first paint; a failure in any
          // one of them must not discard the others or the painted tiles.
          await new Promise<void>((r) => setTimeout(r, 0));
          if (!mounted.current || myGen !== gen.current) return;
          const [php, node, tpl, h] = await Promise.all([
            client.listInstalled("php").catch(() => null),
            client.listInstalled("node").catch(() => null),
            client.getTemplates().catch(() => null),
            client.getHostsStatus().catch(() => null),
          ]);
          if (!mounted.current || myGen !== gen.current) return;
          safeSetSites((prev) => ({
            ...prev,
            phpInstalled: php ?? prev.phpInstalled,
            nodeInstalled: node ?? prev.nodeInstalled,
            templates: tpl ?? prev.templates,
            hosts: h ?? prev.hosts,
          }));

          // Phase 3 — reachability probes (slow, holds the backend state lock).
          if (!probe) return;
          await probeSites();
        } catch {
          if (!mounted.current || myGen !== gen.current) return;
          // Do not clear details on refresh failure.
          safeSetSites((prev) => ({
            ...prev,
            refreshing: false,
            probing: false,
          }));
        } finally {
          if (inflight.current === runPromise) {
            inflight.current = null;
          }
        }
      })();

      inflight.current = runPromise;
      return runPromise;
    },
    [safeSetSites, probeSites],
  );

  const sitesRef = useRef(sites);
  sitesRef.current = sites;

  const ensureSites = useCallback(async () => {
    if (inflight.current) return inflight.current;
    const cur = sitesRef.current;
    if (cur.loaded && !cur.fromSnapshot) return;
    return loadSites({ probe: true });
  }, [loadSites]);

  useEffect(() => {
    mounted.current = true;
    if (!booted.current) {
      booted.current = true;
      // Fast path first (no probe), then probe in the background once painted.
      // A child (SitesPage) effect runs before this one and may already have
      // started a load — never start a second one on top of it.
      if (!inflight.current) {
        void loadSites({ probe: false }).then(() => {
          if (mounted.current) void probeSites();
        });
      }
    }
    return () => {
      mounted.current = false;
      gen.current += 1;
    };
  }, [loadSites, probeSites]);

  // Persist a lightweight snapshot for instant first paint next launch.
  useEffect(() => {
    if (!sites.loaded || sites.fromSnapshot) return;
    writeSitesSnapshot(sites.details, sites.stack);
  }, [sites.loaded, sites.fromSnapshot, sites.details, sites.stack]);

  const value = useMemo<AppDataContextValue>(
    () => ({
      sites,
      loadSites,
      ensureSites,
      probeSites,
      setSitesDetails,
      setStack,
      patchSitesCache,
    }),
    [
      sites,
      loadSites,
      ensureSites,
      probeSites,
      setSitesDetails,
      setStack,
      patchSitesCache,
    ],
  );

  return (
    <AppDataContext.Provider value={value}>{children}</AppDataContext.Provider>
  );
}

export function useAppData(): AppDataContextValue {
  const ctx = useContext(AppDataContext);
  if (!ctx) {
    throw new Error("useAppData must be used within AppDataProvider");
  }
  return ctx;
}
