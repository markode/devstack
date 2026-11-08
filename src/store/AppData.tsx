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

export type AppDataState = {
  siteDetails: SiteDetails[];
  sitesLoaded: boolean;
  sitesRefreshing: boolean;
  stack: StackStatus | null;
  hosts: HostsStatus | null;
  phpInstalled: InstalledPackage[];
  nodeInstalled: InstalledPackage[];
  templates: TemplateRecord[];
  error: string | null;
};

type AppDataContextValue = AppDataState & {
  /** Fast list (+ optional background probe). Never clears cache on failure. */
  refreshSites: (opts?: { probe?: boolean }) => Promise<void>;
  setSiteDetails: (
    next: SiteDetails[] | ((prev: SiteDetails[]) => SiteDetails[]),
  ) => void;
  setStack: (stack: StackStatus | null) => void;
  setHosts: (hosts: HostsStatus | null) => void;
  setTemplates: (templates: TemplateRecord[]) => void;
  setError: (error: string | null) => void;
  refreshInstalled: () => Promise<void>;
};

const AppDataContext = createContext<AppDataContextValue | null>(null);

const initialState: AppDataState = {
  siteDetails: [],
  sitesLoaded: false,
  sitesRefreshing: false,
  stack: null,
  hosts: null,
  phpInstalled: [],
  nodeInstalled: [],
  templates: [],
  error: null,
};

export function AppDataProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<AppDataState>(initialState);
  const refreshGen = useRef(0);
  const booted = useRef(false);

  const setSiteDetails = useCallback(
    (next: SiteDetails[] | ((prev: SiteDetails[]) => SiteDetails[])) => {
      setState((prev) => ({
        ...prev,
        siteDetails: typeof next === "function" ? next(prev.siteDetails) : next,
        sitesLoaded: true,
      }));
    },
    [],
  );

  const setStack = useCallback((stack: StackStatus | null) => {
    setState((prev) => ({ ...prev, stack }));
  }, []);

  const setHosts = useCallback((hosts: HostsStatus | null) => {
    setState((prev) => ({ ...prev, hosts }));
  }, []);

  const setTemplates = useCallback((templates: TemplateRecord[]) => {
    setState((prev) => ({ ...prev, templates }));
  }, []);

  const setError = useCallback((error: string | null) => {
    setState((prev) => ({ ...prev, error }));
  }, []);

  const refreshInstalled = useCallback(async () => {
    try {
      const [php, node] = await Promise.all([
        client.listInstalled("php"),
        client.listInstalled("node"),
      ]);
      setState((prev) => ({
        ...prev,
        phpInstalled: php,
        nodeInstalled: node,
      }));
    } catch {
      /* keep cache */
    }
  }, []);

  const refreshSites = useCallback(async (opts?: { probe?: boolean }) => {
    const gen = ++refreshGen.current;
    setState((prev) => ({
      ...prev,
      sitesRefreshing: true,
      error: null,
    }));
    try {
      const [details, php, node, st, tpl, h] = await Promise.all([
        client.listSiteDetails(),
        client.listInstalled("php"),
        client.listInstalled("node"),
        client.getStackStatus(),
        client.getTemplates(),
        client.getHostsStatus(),
      ]);
      if (gen !== refreshGen.current) return;
      setState((prev) => ({
        ...prev,
        siteDetails: details,
        sitesLoaded: true,
        phpInstalled: php,
        nodeInstalled: node,
        stack: st,
        templates: tpl,
        hosts: h,
        sitesRefreshing: opts?.probe !== false,
      }));

      // Background health probes — never block / clear the fast list.
      if (opts?.probe !== false) {
        void client
          .probeAllSiteReachability()
          .then((probed) => {
            if (gen !== refreshGen.current) return;
            setState((prev) => ({
              ...prev,
              siteDetails: probed,
              sitesLoaded: true,
              sitesRefreshing: false,
            }));
          })
          .catch(() => {
            if (gen !== refreshGen.current) return;
            setState((prev) => ({ ...prev, sitesRefreshing: false }));
          });
      } else {
        setState((prev) => ({ ...prev, sitesRefreshing: false }));
      }
    } catch (e) {
      if (gen !== refreshGen.current) return;
      setState((prev) => ({
        ...prev,
        // Keep existing siteDetails on failure — never flash empty.
        sitesRefreshing: false,
        error: e instanceof Error ? e.message : String(e),
      }));
    }
  }, []);

  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    void refreshSites({ probe: true });
  }, [refreshSites]);

  const value = useMemo<AppDataContextValue>(
    () => ({
      ...state,
      refreshSites,
      setSiteDetails,
      setStack,
      setHosts,
      setTemplates,
      setError,
      refreshInstalled,
    }),
    [
      state,
      refreshSites,
      setSiteDetails,
      setStack,
      setHosts,
      setTemplates,
      setError,
      refreshInstalled,
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
