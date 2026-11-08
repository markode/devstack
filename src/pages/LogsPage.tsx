import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import * as client from "../client";
import type { LogEntry, LogLevel } from "../types";

const ROW_H = 22;
const SOURCES = [
  "all",
  "app",
  "command",
  "ui",
  "console",
  "nginx",
  "apache",
  "php",
  "db",
  "mailpit",
  "redis",
  "hosts",
  "process",
] as const;

export function LogsPage({ active }: { active?: boolean }) {
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [level, setLevel] = useState<"all" | LogLevel>("all");
  const [source, setSource] = useState<(typeof SOURCES)[number]>("all");
  const [query, setQuery] = useState("");
  const [paused, setPaused] = useState(false);
  const [scrollTop, setScrollTop] = useState(0);
  const viewportRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);
  const [viewportH, setViewportH] = useState(400);

  const refresh = useCallback(async () => {
    const list = await client.getAppLogs(5000);
    setEntries(list);
  }, []);

  useEffect(() => {
    void refresh();
    let unlisten = () => {};
    void client.onAppLog((entry) => {
      setEntries((prev) => {
        const next = [...prev, entry];
        if (next.length > 5000) next.splice(0, next.length - 5000);
        return next;
      });
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten();
  }, [refresh]);

  useEffect(() => {
    if (!active) return;
    void client.clearAppLogErrorCount();
  }, [active]);

  useEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewportH(el.clientHeight));
    ro.observe(el);
    setViewportH(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (paused || !stickRef.current) return;
    const el = viewportRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [entries, paused]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return entries.filter((e) => {
      if (level !== "all" && e.level !== level) return false;
      if (source !== "all" && e.source !== source) return false;
      if (!q) return true;
      return (
        e.message.toLowerCase().includes(q) ||
        e.source.toLowerCase().includes(q) ||
        e.level.includes(q)
      );
    });
  }, [entries, level, source, query]);

  const totalH = filtered.length * ROW_H;
  const start = Math.max(0, Math.floor(scrollTop / ROW_H) - 5);
  const visibleCount = Math.ceil(viewportH / ROW_H) + 10;
  const slice = filtered.slice(start, start + visibleCount);

  const onScroll = () => {
    const el = viewportRef.current;
    if (!el) return;
    setScrollTop(el.scrollTop);
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    stickRef.current = atBottom;
    if (!atBottom && !paused) setPaused(true);
    if (atBottom && paused) setPaused(false);
  };

  const copyAll = async () => {
    const text = filtered
      .map((e) => `${e.ts}\t${e.level}\t${e.source}\t${e.message}`)
      .join("\n");
    await navigator.clipboard.writeText(text);
  };

  return (
    <div className="logs-page" data-testid="logs-page">
      <div className="logs-toolbar">
        <label className="logs-filter">
          Level
          <select
            value={level}
            onChange={(e) => setLevel(e.target.value as "all" | LogLevel)}
            data-testid="logs-level"
          >
            <option value="all">All</option>
            <option value="info">Info</option>
            <option value="warn">Warn</option>
            <option value="error">Error</option>
          </select>
        </label>
        <label className="logs-filter">
          Source
          <select
            value={source}
            onChange={(e) =>
              setSource(e.target.value as (typeof SOURCES)[number])
            }
            data-testid="logs-source"
          >
            {SOURCES.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
        </label>
        <input
          className="logs-search"
          placeholder="Search…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          data-testid="logs-search"
        />
        <span className="muted logs-meta">
          {filtered.length}/{entries.length}
          {paused ? " · paused" : " · live"}
        </span>
        <div className="logs-actions">
          <button
            type="button"
            className="btn btn-install"
            onClick={() => {
              stickRef.current = true;
              setPaused(false);
              const el = viewportRef.current;
              if (el) el.scrollTop = el.scrollHeight;
            }}
          >
            Resume
          </button>
          <button
            type="button"
            className="btn btn-install"
            onClick={() => void copyAll()}
          >
            Copy all
          </button>
          <button
            type="button"
            className="btn btn-install"
            onClick={() => void client.openAppLogFolder()}
          >
            Open log folder
          </button>
          <button
            type="button"
            className="btn btn-install"
            onClick={() => void client.clearAppLogs().then(refresh)}
          >
            Clear
          </button>
        </div>
      </div>
      <div
        className="logs-viewport"
        ref={viewportRef}
        onScroll={onScroll}
        data-testid="logs-viewport"
      >
        <div className="logs-spacer" style={{ height: totalH }}>
          <div
            className="logs-window"
            style={{ transform: `translateY(${start * ROW_H}px)` }}
          >
            {slice.map((e) => (
              <div
                key={e.id}
                className={`logs-row level-${e.level}`}
                style={{ height: ROW_H }}
                title={e.message}
              >
                <span className="logs-ts">{e.ts.slice(11, 23)}</span>
                <span className="logs-level">{e.level}</span>
                <span className="logs-source">{e.source}</span>
                <span className="logs-msg">{e.message}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
