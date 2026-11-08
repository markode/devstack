import { useEffect, useState, type ReactNode } from "react";
import { PageHeader } from "./components/PageHeader";
import { Sidebar } from "./components/Sidebar";
import { GeneralPage, PlaceholderPage } from "./pages/GeneralPage";
import { PhpPage } from "./pages/PhpPage";
import { JavaScriptPage } from "./pages/JavaScriptPage";
import { ServicesPage } from "./pages/ServicesPage";
import { SitesPage } from "./pages/SitesPage";
import { LogsPage } from "./pages/LogsPage";
import { AboutPage } from "./pages/AboutPage";
import { AppDataProvider, useAppData } from "./store/AppData";
import * as client from "./client";
import type { DeveloperSettings, SectionId } from "./types";

const TITLES: Record<SectionId, string> = {
  general: "General",
  php: "PHP",
  javascript: "JavaScript",
  services: "Services",
  sites: "Sites",
  logs: "Logs",
  ai: "AI",
  pro: "Pro",
  sharing: "Sharing",
  shortcuts: "Shortcuts",
  about: "About",
};

/** Sections kept mounted (hidden) after first visit so local + cached state survives. */
const KEEP_ALIVE: SectionId[] = ["sites", "services", "php", "javascript", "logs"];

const SECTIONS = Object.keys(TITLES) as SectionId[];

function sectionFromHash(): SectionId {
  const raw = window.location.hash.replace(/^#\/?/, "").toLowerCase();
  const top = raw.split("/")[0] || "";
  if (SECTIONS.includes(top as SectionId)) return top as SectionId;
  return "php";
}

/** Optional deep-link: #/sites/<siteId> */
export function siteIdFromHash(): string | null {
  const raw = window.location.hash.replace(/^#\/?/, "");
  const parts = raw.split("/");
  if (parts[0]?.toLowerCase() === "sites" && parts[1]) return parts[1];
  return null;
}

function KeepAlivePane({
  id,
  active,
  visited,
  children,
}: {
  id: SectionId;
  active: SectionId;
  visited: ReadonlySet<SectionId>;
  children: ReactNode;
}) {
  if (!visited.has(id) && active !== id) return null;
  return (
    <div
      className="page-pane"
      data-section={id}
      data-testid={`pane-${id}`}
      hidden={active !== id}
      aria-hidden={active !== id}
    >
      {children}
    </div>
  );
}

function AppShell() {
  const [section, setSection] = useState<SectionId>(sectionFromHash);
  const [siteDeepLink, setSiteDeepLink] = useState<string | null>(siteIdFromHash);
  const [visited, setVisited] = useState<Set<SectionId>>(
    () => new Set<SectionId>([sectionFromHash()]),
  );
  const [developer, setDeveloper] = useState<DeveloperSettings>({
    showLogsTab: true,
    redactSecrets: false,
  });
  const [logErrors, setLogErrors] = useState(0);
  const { sites } = useAppData();
  const sitesBusy = sites.refreshing || sites.probing;

  useEffect(() => {
    void client.getDeveloperSettings().then(setDeveloper).catch(() => {});
    void client.getAppLogErrorCount().then(setLogErrors).catch(() => {});
    let unlisten = () => {};
    void client
      .onAppLog((entry) => {
        if (entry.level === "error") {
          setLogErrors((n) => n + 1);
        }
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => unlisten();
  }, []);

  useEffect(() => {
    const onHash = () => {
      const next = sectionFromHash();
      setSection(next);
      setSiteDeepLink(siteIdFromHash());
      setVisited((prev) => {
        if (prev.has(next)) return prev;
        const copy = new Set(prev);
        copy.add(next);
        return copy;
      });
    };
    window.addEventListener("hashchange", onHash);
    if (!window.location.hash) {
      window.location.hash = `#/${section}`;
    }
    return () => window.removeEventListener("hashchange", onHash);
  }, [section]);

  useEffect(() => {
    setVisited((prev) => {
      if (prev.has(section)) return prev;
      const copy = new Set(prev);
      copy.add(section);
      return copy;
    });
  }, [section]);

  const select = (id: SectionId) => {
    setSection(id);
    setVisited((prev) => {
      if (prev.has(id)) return prev;
      const copy = new Set(prev);
      copy.add(id);
      return copy;
    });
    window.location.hash = `#/${id}`;
  };

  const ephemeral = !KEEP_ALIVE.includes(section);

  const selectLogged = (id: SectionId) => {
    void client
      .appendAppLog("info", "ui", `sidebar select ${id}`)
      .catch(() => {});
    if (id === "logs") setLogErrors(0);
    select(id);
  };

  return (
    <div className="app-shell">
      <Sidebar
        active={section}
        onSelect={selectLogged}
        showLogsTab={developer.showLogsTab}
        logErrorCount={section === "logs" ? 0 : logErrors}
      />
      <main className="content">
        <PageHeader
          title={TITLES[section]}
          meta={
            section === "sites" && sites.loaded
              ? `${sites.details.length} ${
                  sites.details.length === 1 ? "site" : "sites"
                }`
              : undefined
          }
          refreshing={sitesBusy}
        />
        <div className="content-body">
          <KeepAlivePane id="sites" active={section} visited={visited}>
            <SitesPage
              initialSiteId={siteDeepLink}
              active={section === "sites"}
            />
          </KeepAlivePane>
          <KeepAlivePane id="services" active={section} visited={visited}>
            <ServicesPage />
          </KeepAlivePane>
          <KeepAlivePane id="php" active={section} visited={visited}>
            <PhpPage />
          </KeepAlivePane>
          <KeepAlivePane id="javascript" active={section} visited={visited}>
            <JavaScriptPage />
          </KeepAlivePane>
          <KeepAlivePane id="logs" active={section} visited={visited}>
            <LogsPage active={section === "logs"} />
          </KeepAlivePane>
          {ephemeral ? (
            <>
              {section === "general" && (
                <GeneralPage onDeveloperChange={setDeveloper} />
              )}
              {section === "ai" && (
                <PlaceholderPage
                  title="AI"
                  blurb="Placeholder — assistant integrations arrive in a later milestone."
                />
              )}
              {section === "pro" && (
                <PlaceholderPage
                  title="Pro"
                  blurb="Placeholder — Pro features will land here."
                />
              )}
              {section === "sharing" && (
                <PlaceholderPage
                  title="Sharing"
                  blurb="Placeholder — public URL sharing for local sites."
                />
              )}
              {section === "shortcuts" && (
                <PlaceholderPage
                  title="Shortcuts"
                  blurb="Placeholder — keyboard shortcuts and CLI helpers."
                />
              )}
              {section === "about" && <AboutPage />}
            </>
          ) : null}
        </div>
      </main>
    </div>
  );
}

export default function App() {
  return (
    <AppDataProvider>
      <AppShell />
    </AppDataProvider>
  );
}
