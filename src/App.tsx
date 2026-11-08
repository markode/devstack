import { useEffect, useState, type ReactNode } from "react";
import { Sidebar } from "./components/Sidebar";
import { GeneralPage, PlaceholderPage } from "./pages/GeneralPage";
import { PhpPage } from "./pages/PhpPage";
import { JavaScriptPage } from "./pages/JavaScriptPage";
import { ServicesPage } from "./pages/ServicesPage";
import { SitesPage } from "./pages/SitesPage";
import { AboutPage } from "./pages/AboutPage";
import { AppDataProvider, useAppData } from "./store/AppData";
import type { SectionId } from "./types";

const TITLES: Record<SectionId, string> = {
  general: "General",
  php: "PHP",
  javascript: "JavaScript",
  services: "Services",
  sites: "Sites",
  ai: "AI",
  pro: "Pro",
  sharing: "Sharing",
  shortcuts: "Shortcuts",
  about: "About",
};

/** Sections kept mounted (hidden) after first visit so local + cached state survives. */
const KEEP_ALIVE: SectionId[] = ["sites", "services", "php", "javascript"];

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
  const { sites } = useAppData();
  const sitesBusy = sites.refreshing || sites.probing;

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

  return (
    <div className="app-shell">
      <Sidebar active={section} onSelect={select} />
      <main className="content">
        <header className="content-header">
          <div className="nav-arrows" aria-hidden>
            ‹ ›
          </div>
          <div className="content-title">{TITLES[section]}</div>
          {section === "sites" && sites.loaded ? (
            <div className="content-header-meta" aria-live="polite">
              {sites.details.length}{" "}
              {sites.details.length === 1 ? "site" : "sites"}
            </div>
          ) : null}
          {sitesBusy ? (
            <div
              className="content-header-loader"
              role="status"
              aria-label="Refreshing"
              data-testid="sites-header-loader"
            />
          ) : null}
        </header>
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
          {ephemeral ? (
            <>
              {section === "general" && <GeneralPage />}
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
