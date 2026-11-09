import { useCallback, useEffect, useRef, useState } from "react";
import * as client from "../client";
import { humanizeError } from "../portBusy";
import { openInBrowser } from "../open";
import type { LogSourceInfo, QueueStatus, SiteLogEntry, SiteLogSource } from "../types";
import { mergePolledEntries } from "../v030";

const LEVELS = ["all", "debug", "info", "notice", "warning", "error", "critical"];
const SINCE = [
  { value: "", label: "Any time" },
  { value: "15m", label: "Last 15 min" },
  { value: "1h", label: "Last hour" },
  { value: "1d", label: "Last day" },
];

/** Filterable per-site logs with live polling + Laravel queue panel. */
export function SiteLogsPanel({ siteId }: { siteId: string }) {
  const [sources, setSources] = useState<LogSourceInfo[]>([]);
  const [source, setSource] = useState<SiteLogSource>("php");
  const [level, setLevel] = useState("all");
  const [text, setText] = useState("");
  const [since, setSince] = useState("");
  const [live, setLive] = useState(false);
  const [entries, setEntries] = useState<SiteLogEntry[]>([]);
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const cursor = useRef<number | null>(null);

  useEffect(() => {
    void client
      .listSiteLogSources(siteId)
      .then((s) => {
        setSources(s);
        if (!s.some((x) => x.id === source)) setSource(s[0]?.id ?? "php");
      })
      .catch((e) => setError(humanizeError(e instanceof Error ? e.message : String(e))));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [siteId]);

  const query = useCallback(
    (withCursor: boolean) => ({
      source,
      level: level === "all" ? undefined : level,
      text: text || undefined,
      since: since || undefined,
      lines: 300,
      cursor: withCursor ? cursor.current : null,
    }),
    [source, level, text, since],
  );

  const reload = useCallback(async () => {
    setError(null);
    try {
      const page = await client.readSiteLogs(siteId, query(false));
      cursor.current = page.cursor;
      setPath(page.path);
      setEntries(page.entries);
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    }
  }, [siteId, query]);

  useEffect(() => {
    cursor.current = null;
    void reload();
  }, [reload]);

  useEffect(() => {
    if (!live) return;
    const t = window.setInterval(() => {
      void client
        .readSiteLogs(siteId, query(true))
        .then((page) => {
          cursor.current = page.cursor;
          if (page.entries.length > 0 || page.reset) {
            setEntries((cur) => mergePolledEntries(cur, page.entries, 1000, page.reset));
          }
        })
        .catch(() => {});
    }, 2000);
    return () => window.clearInterval(t);
  }, [live, siteId, query]);

  return (
    <>
      <div className="panel-card settings-list" style={{ marginTop: 16 }} data-testid="site-logs-panel">
        <div className="settings-row settings-row-head">
          <label>
            Logs
            <span className="hint">{path || "—"}</span>
          </label>
          <div className="row-actions">
            <label className="chip chip-toggle" style={{ cursor: "pointer" }}>
              <input
                type="checkbox"
                checked={live}
                onChange={(e) => setLive(e.target.checked)}
                aria-label="Live tail"
              />{" "}
              Live
            </label>
            <button type="button" className="btn btn-install" onClick={() => void reload()}>
              Refresh
            </button>
          </div>
        </div>
        <div className="settings-row settings-row-block">
          <div className="field-row">
            <select className="native" aria-label="Log source" value={source} onChange={(e) => setSource(e.target.value as SiteLogSource)}>
              {sources.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.label}
                  {s.exists ? "" : " (empty)"}
                </option>
              ))}
            </select>
            <select className="native" aria-label="Level" value={level} onChange={(e) => setLevel(e.target.value)}>
              {LEVELS.map((l) => (
                <option key={l} value={l}>
                  {l === "all" ? "All levels" : `≥ ${l}`}
                </option>
              ))}
            </select>
            <select className="native" aria-label="Time range" value={since} onChange={(e) => setSince(e.target.value)}>
              {SINCE.map((s) => (
                <option key={s.value} value={s.value}>
                  {s.label}
                </option>
              ))}
            </select>
            <input placeholder="Filter text…" aria-label="Filter text" value={text} onChange={(e) => setText(e.target.value)} />
          </div>
          {error ? <span className="error-inline">{error}</span> : null}
          <div className="site-log-list" data-testid="site-log-entries">
            {entries.length === 0 ? (
              <span className="muted">No matching entries.</span>
            ) : (
              entries.map((e, i) => (
                <div key={`${i}-${e.ts ?? ""}`} className={`site-log-entry lvl-${e.level}`}>
                  <span className="site-log-ts">{e.ts ?? ""}</span>
                  <span className="site-log-level">{e.level}</span>
                  <pre className="site-log-msg">{e.message}</pre>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
      <SiteQueuePanel siteId={siteId} />
    </>
  );
}

export function SiteQueuePanel({ siteId }: { siteId: string }) {
  const [status, setStatus] = useState<QueueStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => {
    try {
      setStatus(await client.getSiteQueueStatus(siteId));
      setError(null);
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    }
  }, [siteId]);
  useEffect(() => {
    void load();
  }, [load]);

  if (!status?.isLaravel) return null;
  return (
    <div className="panel-card settings-list" style={{ marginTop: 16 }} data-testid="site-queue-panel">
      <div className="settings-row settings-row-head">
        <label>
          Laravel queue
          <span className="hint">driver: {status.driver ?? "unknown"}</span>
        </label>
        <div className="row-actions">
          {status.horizon.installed && status.horizon.url ? (
            <button
              type="button"
              className="btn btn-install"
              onClick={() => void openInBrowser(status.horizon.url!).catch(() => {})}
            >
              Open Horizon
            </button>
          ) : null}
          <button type="button" className="btn btn-install" onClick={() => void load()}>
            Refresh
          </button>
        </div>
      </div>
      <div className="settings-row">
        <label>Pending jobs</label>
        <strong>{status.pending ?? "—"}</strong>
      </div>
      <div className="settings-row">
        <label>Failed jobs</label>
        <strong>{status.failed ?? "—"}</strong>
      </div>
      {status.failedJobs.length > 0 ? (
        <div className="settings-row settings-row-block">
          {status.failedJobs.map((j) => (
            <div key={j.id} className="muted" style={{ fontSize: 12 }}>
              #{j.id} · {j.queue} · {j.failedAt} — {j.exception}
            </div>
          ))}
        </div>
      ) : null}
      {status.notes.length > 0 || error ? (
        <div className="settings-row settings-row-block">
          {[...status.notes, ...(error ? [error] : [])].map((n) => (
            <span key={n} className="hint">
              {n}
            </span>
          ))}
        </div>
      ) : null}
    </div>
  );
}
