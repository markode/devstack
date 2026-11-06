import { useEffect, useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { GeneralPage, PlaceholderPage } from "./pages/GeneralPage";
import { PhpPage } from "./pages/PhpPage";
import { JavaScriptPage } from "./pages/JavaScriptPage";
import { ServicesPage } from "./pages/ServicesPage";
import { SitesPage } from "./pages/SitesPage";
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

const SECTIONS = Object.keys(TITLES) as SectionId[];

function sectionFromHash(): SectionId {
  const raw = window.location.hash.replace(/^#\/?/, "").toLowerCase();
  if (SECTIONS.includes(raw as SectionId)) return raw as SectionId;
  return "php";
}

export default function App() {
  const [section, setSection] = useState<SectionId>(sectionFromHash);

  useEffect(() => {
    const onHash = () => setSection(sectionFromHash());
    window.addEventListener("hashchange", onHash);
    if (!window.location.hash) {
      window.location.hash = `#/${section}`;
    }
    return () => window.removeEventListener("hashchange", onHash);
  }, [section]);

  const select = (id: SectionId) => {
    setSection(id);
    window.location.hash = `#/${id}`;
  };

  return (
    <div className="app-shell">
      <Sidebar active={section} onSelect={select} />
      <main className="content">
        <header className="content-header">
          <div className="nav-arrows" aria-hidden>
            ‹ ›
          </div>
          <div className="content-title">{TITLES[section]}</div>
        </header>
        <div className="content-body" key={section}>
          {section === "general" && <GeneralPage />}
          {section === "php" && <PhpPage />}
          {section === "javascript" && <JavaScriptPage />}
          {section === "services" && <ServicesPage />}
          {section === "sites" && <SitesPage />}
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
          {section === "about" && (
            <PlaceholderPage
              title="About"
              blurb="DevStack milestone 1 — Tauri 2 + Rust package core + React UI."
            />
          )}
        </div>
      </main>
    </div>
  );
}
