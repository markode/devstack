import type { SectionId } from "../types";

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
  { id: "ai", label: "AI", color: "#af52de", glyph: "A", placeholder: true },
  { id: "pro", label: "Pro", color: "#ff9500", glyph: "★", placeholder: true },
  {
    id: "sharing",
    label: "Sharing",
    color: "#30b0c7",
    glyph: "↗",
    placeholder: true,
  },
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
}: {
  active: SectionId;
  onSelect: (id: SectionId) => void;
}) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark">DS</div>
        <div className="brand-name">DevStack</div>
      </div>
      {ITEMS.map((item) => (
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
        </button>
      ))}
    </aside>
  );
}
