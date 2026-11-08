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
};

type AppDataContextValue = {
  sites: SitesCache;
  /** Load fast list immediately; optionally probe in background. Never clears cache on failure. */
  loadSites: (opts?: { force?: boolean; probe?: boolean }) => Promise<void>;
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
};

const AppDataContext = createContext<AppDataContextValue | null>(null);

export function AppDataProvider({ children }: { children: ReactNode }) {
  const [sites, setSites] = useState<SitesCache>(emptySites);
  const gen = useRef(0);
  const inflight = useRef<Promise<void> | null>(null);
  const booted = useRef(false);

  const patchSitesCache = useCallback((patch: Partial<SitesCache>) => {
    setSites((prev) => ({ ...prev, ...patch }));
  }, []);

  const setSitesDetails = useCallback(
    (next: SiteDetails[] | ((prev: SiteDetails[]) => SiteDetails[])) => {
      setSites((prev) => ({
        ...prev,
        details: typeof next === "function" ? next(prev.details) : next,
        loaded: true,
      }));
    },
    [],
  );

  const setStack = useCallback((stack: StackStatus | null) => {
    setSites((prev) => ({ ...prev, stack }));
  }, []);

  const loadSites = useCallback(
    async (opts?: { force?: boolean; probe?: boolean }) => {
      const probe = opts?.probe !== false;
      // Deduplicate concurrent loads unless forced.
      if (inflight.current && !opts?.force) {
        return inflight.current;
      }

      const myGen = ++gen.current;
      setSites((prev) => ({
        ...prev,
        refreshing: true,
        // Keep existing details visible — never clear to [].
      }));

      let runPromise!: Promise<void>;
      runPromise = (async () => {
        try {
          const [details, php, node, st, tpl, h] = await Promise.all([
            client.listSiteDetails(),
            client.listInstalled("php"),
            client.listInstalled("node"),
            client.getStackStatus(),
            client.getTemplates(),
            client.getHostsStatus(),
          ]);
          if (myGen !== gen.current) return;
          setSites((prev) => ({
            ...prev,
            details,
            phpInstalled: php,
            nodeInstalled: node,
            stack: st,
            templates: tpl,
            hosts: h,
            loaded: true,
            refreshing: false,
            probing: probe,
          }));

          if (!probe) return;

          try {
            const probed = await client.probeAllSiteReachability();
            if (myGen !== gen.current) return;
            setSites((prev) => ({
              ...prev,
              details: probed,
              loaded: true,
              probing: false,
            }));
          } catch {
            if (myGen !== gen.current) return;
            // Keep fast-list cache; just stop the top loader.
            setSites((prev) => ({ ...prev, probing: false }));
          }
        } catch {
          if (myGen !== gen.current) return;
          // Do not clear details on refresh failure.
          setSites((prev) => ({
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
    [],
  );

  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    void loadSites({ probe: true });
  }, [loadSites]);

  const value = useMemo<AppDataContextValue>(
    () => ({
      sites,
      loadSites,
      setSitesDetails,
      setStack,
      patchSitesCache,
    }),
    [sites, loadSites, setSitesDetails, setStack, patchSitesCache],
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
