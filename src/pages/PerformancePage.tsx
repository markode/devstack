import { useCallback, useEffect, useRef, useState } from "react";
import { SectionHeader } from "../components/SectionHeader";
import * as client from "../client";
import { humanizeError } from "../portBusy";
import { openInFileManager } from "../open";
import type { MetricsSnapshot, SlowQueryInfo, SpxStatus } from "../types";
import {
  formatBytes,
  formatPercent,
  pushHistory,
  sortProcesses,
  sparklinePoints,
} from "../v030";

/** CPU/RAM per managed process, slow-query log and SPX profiler. */
export function PerformancePage({ active = true }: { active?: boolean }) {
  const [snap, setSnap] = useState<MetricsSnapshot | null>(null);
  const [cpuHistory, setCpuHistory] = useState<number[]>([]);
  const [memHistory, setMemHistory] = useState<number[]>([]);
  const [slow, setSlow] = useState<SlowQueryInfo | null>(null);
  const [spx, setSpx] = useState<SpxStatus | null>(null);
  const [threshold, setThreshold] = useState(1);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const mounted = useRef(true);

  const sample = useCallback(async () => {
    try {
      const s = await client.getMetricsSnapshot();
      if (!mounted.current) return;
      setSnap(s);
      setCpuHistory((h) => pushHistory(h, s.totalCpuPercent));
      setMemHistory((h) => pushHistory(h, s.totalMemoryBytes / (1024 * 1024)));
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    }
  }, []);

  const loadDetails = useCallback(async () => {
    const [a, b] = await Promise.allSettled([client.getSlowQueryInfo(50), client.getSpxStatus()]);
    if (!mounted.current) return;
    if (a.status === "fulfilled") {
      setSlow(a.value);
      setThreshold(a.value.thresholdSecs || 1);
    }
    if (b.status === "fulfilled") setSpx(b.value);
  }, []);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (!active) return;
    void sample();
    void loadDetails();
    const t = window.setInterval(() => void sample(), 3000);
    return () => window.clearInterval(t);
  }, [active, sample, loadDetails]);

  const guard = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
    }
  };

  const rows = snap ? sortProcesses(snap.processes) : [];

  return (
    <div data-testid="performance-page">
      <SectionHeader
        title="Performance"
        description="Live CPU and memory for every DevStack-managed process, slow queries and profiling."
      />
      {error ? <div className="error-banner">{error}</div> : null}

      <div className="perf-summary">
        <div className="panel-card perf-card">
          <span className="muted">Stack CPU</span>
          <strong>{snap ? formatPercent(snap.totalCpuPercent) : "—"}</strong>
          <svg viewBox="0 0 120 28" className="sparkline" aria-hidden>
            <polyline fill="none" stroke="currentColor" strokeWidth="1.5" points={sparklinePoints(cpuHistory, 120, 28)} />
          </svg>
        </div>
        <div className="panel-card perf-card">
          <span className="muted">Stack memory</span>
          <strong>{snap ? formatBytes(snap.totalMemoryBytes) : "—"}</strong>
          <svg viewBox="0 0 120 28" className="sparkline" aria-hidden>
            <polyline fill="none" stroke="currentColor" strokeWidth="1.5" points={sparklinePoints(memHistory, 120, 28)} />
          </svg>
        </div>
        <div className="panel-card perf-card">
          <span className="muted">System</span>
          <strong>
            {snap ? `${formatPercent(snap.system.cpuPercent)} · ${snap.system.cpuCount} cores` : "—"}
          </strong>
          <span className="muted">
            {snap
              ? `${formatBytes(snap.system.usedMemoryBytes)} / ${formatBytes(snap.system.totalMemoryBytes)} RAM`
              : ""}
          </span>
        </div>
      </div>

      <div className="panel-card settings-list" style={{ marginTop: 16 }}>
        <div className="settings-row settings-row-head">
          <label>Managed processes</label>
          <button type="button" className="btn btn-install" onClick={() => void sample()}>
            Refresh
          </button>
        </div>
        {rows.length === 0 ? (
          <div className="settings-row">
            <span className="muted">Nothing running — start the stack or a service.</span>
          </div>
        ) : (
          <table className="perf-table" data-testid="perf-processes">
            <thead>
              <tr>
                <th>Process</th>
                <th>PID</th>
                <th>CPU</th>
                <th>Memory</th>
                <th>Tree</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((p) => (
                <tr key={`${p.id}-${p.pid}`} className={p.running ? "" : "muted"}>
                  <td>
                    <span className={`status-dot ${p.running ? "on" : ""}`} /> {p.label}
                  </td>
                  <td>{p.pid}</td>
                  <td>{p.running ? formatPercent(p.cpuPercent) : "—"}</td>
                  <td>{p.running ? formatBytes(p.memoryBytes) : "—"}</td>
                  <td>{p.running ? p.processes : "stopped"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      <div className="panel-card settings-list" style={{ marginTop: 16 }} data-testid="slow-query-panel">
        <div className="settings-row settings-row-head">
          <label>
            Slow query log
            <span className="hint">
              {slow?.engine ?? "no engine"} · {slow?.path ?? ""}
            </span>
          </label>
          <div className="row-actions">
            <input
              type="number"
              min={0}
              step={0.5}
              aria-label="Threshold seconds"
              value={threshold}
              style={{ width: 80 }}
              onChange={(e) => setThreshold(Number(e.target.value))}
            />
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy || !slow?.running}
              onClick={() =>
                void guard(async () => setSlow(await client.setSlowQueryLog(!slow?.enabled, threshold)))
              }
            >
              {slow?.enabled ? "Disable" : "Enable"}
            </button>
          </div>
        </div>
        {slow?.notes.map((n) => (
          <div key={n} className="settings-row">
            <span className="hint">{n}</span>
          </div>
        ))}
        {slow && slow.entries.length === 0 ? (
          <div className="settings-row">
            <span className="muted">{slow.enabled ? "No slow queries recorded yet." : "Logging is off."}</span>
          </div>
        ) : null}
        {slow?.entries.map((q, i) => (
          <div key={`${q.time}-${i}`} className="settings-row settings-row-block">
            <span className="hint">
              {q.time} · {q.queryTime.toFixed(2)}s · {q.rowsExamined} rows examined · {q.userHost}
            </span>
            <pre className="code-block">{q.sql}</pre>
          </div>
        ))}
      </div>

      <div className="panel-card settings-list" style={{ marginTop: 16 }} data-testid="spx-panel">
        <div className="settings-row settings-row-head">
          <label>
            SPX profiler
            <span className="hint">PHP {spx?.phpVersion ?? "—"}</span>
          </label>
          <div className="row-actions">
            <button
              type="button"
              className="btn btn-install"
              disabled={!spx}
              onClick={() => spx && void openInFileManager(spx.reportDir).catch((e) => setError(humanizeError(String(e))))}
            >
              Open report folder
            </button>
            {!spx?.available && !spx?.suggestXdebugProfiling ? (
              <button
                type="button"
                className="btn btn-install"
                disabled={busy || !spx}
                onClick={() =>
                  void guard(async () => setSpx(await client.ensureSpxExtension()))
                }
              >
                Install SPX
              </button>
            ) : null}
            {spx?.suggestXdebugProfiling ? (
              <button
                type="button"
                className="btn btn-primary"
                disabled={busy || !spx}
                onClick={() =>
                  void guard(async () =>
                    setSpx(await client.enableXdebugProfiling()),
                  )
                }
              >
                Use Xdebug profiling instead
              </button>
            ) : (
              <button
                type="button"
                className="btn btn-primary"
                disabled={busy || !spx || (!spx.available && !spx.enabled)}
                onClick={() => void guard(async () => setSpx(await client.setSpxEnabled(!spx?.enabled)))}
              >
                {spx?.enabled ? "Disable" : "Enable"}
              </button>
            )}
          </div>
        </div>
        <div className="settings-row settings-row-block">
          <span className="hint">
            {spx?.available
              ? `Add ${spx.uiQuery} to any site URL to open the SPX control panel.`
              : spx?.suggestXdebugProfiling
                ? "SPX has no Windows builds — enable Xdebug profiling as the supported alternative."
                : "Install the php-spx extension to enable profiling."}
          </span>
          {spx?.notes.map((n) => (
            <span key={n} className="hint">
              {n}
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}
