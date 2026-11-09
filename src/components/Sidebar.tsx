import type { SectionId } from "../types";
import type { AppPlatform } from "../platform";

interface NavItem {
  id: SectionId;
  label: string;
  color: string;
  glyph: string;
  placeholder?: boolean;
}

const ITEMS: NavItem[] = [
  { id: "general", label: "General", color: "#8e8e93", glyph: "G" },
  { id: "php", label: "PHP", color: "#777BB4", glyph: "P" },
  { id: "javascript", label: "JavaScript", color: "#3C873A", glyph: "N" },
  { id: "services", label: "Services", color: "#5856d6", glyph: "S" },
  { id: "sites", label: "Sites", color: "#007aff", glyph: "W" },
  { id: "mail", label: "Mail", color: "#0a84ff", glyph: "M" },
  { id: "dumps", label: "Dumps", color: "#ff6b35", glyph: "D" },
  { id: "backups", label: "Backups", color: "#30a46c", glyph: "B" },
  { id: "logs", label: "Logs", color: "#34c759", glyph: "L" },
  { id: "ai", label: "AI", color: "#af52de", glyph: "A", placeholder: true },
  { id: "pro", label: "Pro", color: "#ff9500", glyph: "★", placeholder: true },
  { id: "sharing", label: "Sharing", color: "#30b0c7", glyph: "↗" },
  {
    id: "shortcuts",
    label: "Shortcuts",
    color: "#ff2d55",
    glyph: "⌘",
    placeholder: true,
  },
  { id: "about", label: "About", color: "#64d2ff", glyph: "i" },
];

export function Sidebar({
  active,
  onSelect,
  showLogsTab = true,
  logErrorCount = 0,
  platform = "web",
}: {
  active: SectionId;
  onSelect: (id: SectionId) => void;
  showLogsTab?: boolean;
  logErrorCount?: number;
  platform?: AppPlatform;
}) {
  return (
    <aside
      className={`sidebar platform-${platform}`}
      data-testid="sidebar"
      onContextMenu={(e) => e.preventDefault()}
    >
      <div
        className="sidebar-top"
        data-tauri-drag-region
        data-testid="sidebar-drag"
      >
        {platform === "macos" ? (
          <div className="traffic-light-spacer" aria-hidden />
        ) : null}
        <div className="brand" data-tauri-drag-region>
          <div className="brand-mark">DS</div>
          <div className="brand-name">DevStack</div>
        </div>
      </div>
      <nav className="sidebar-nav" aria-label="Main">
        {ITEMS.filter((item) => item.id !== "logs" || showLogsTab).map(
          (item) => (
            <button
              key={item.id}
              type="button"
              className={`nav-item ${active === item.id ? "active" : ""} ${
                item.placeholder ? "muted" : ""
              }`}
              onClick={() => onSelect(item.id)}
            >
              <span className="nav-icon" style={{ background: item.color }}>
                {item.glyph}
              </span>
              <span className="nav-label">{item.label}</span>
              {item.id === "logs" && logErrorCount > 0 ? (
                <span className="nav-badge" data-testid="logs-error-badge">
                  {logErrorCount > 99 ? "99+" : logErrorCount}
                </span>
              ) : null}
            </button>
          ),
        )}
      </nav>
    </aside>
  );
}
