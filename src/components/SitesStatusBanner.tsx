import type { JobProgress, StackStatus } from "../types";

export type StackBannerState = "stopped" | "starting" | "running";

export function bannerState(running: boolean, starting: boolean): StackBannerState {
  if (starting) return "starting";
  return running ? "running" : "stopped";
}

export function runningSummary(stack: StackStatus | null): string {
  const parts = [
    stack?.nginx ? "Nginx" : null,
    stack?.apache ? "Apache" : null,
    stack?.database ? (stack.databaseEngine ?? "Database") : null,
  ].filter(Boolean);
  return parts.length ? `Running · ${parts.join(" · ")}` : "Running";
}

function phasePercent(p: JobProgress): number | null {
  if (p.percent != null) return Math.max(0, Math.min(100, p.percent));
  if (p.total && p.total > 0) {
    return Math.max(0, Math.min(100, (p.bytes / p.total) * 100));
  }
  return null;
}

/**
 * Primary stack signal for Sites: large when stopped, per-service phases while
 * starting, a compact dot + text once running.
 */
export function SitesStatusBanner({
  running,
  starting,
  phases,
  stack,
  busy = false,
  onStart,
  onStop,
}: {
  running: boolean;
  starting: boolean;
  /** Latest job-progress event per phase, in arrival order. */
  phases: JobProgress[];
  stack: StackStatus | null;
  busy?: boolean;
  onStart: () => void;
  onStop?: () => void;
}) {
  const state = bannerState(running, starting);

  if (state === "running") {
    return (
      <div
        className="stack-banner stack-banner-compact"
        data-testid="stack-banner"
        data-state="running"
        role="status"
      >
        <span className="status-dot on" aria-hidden />
        <span className="stack-banner-text">{runningSummary(stack)}</span>
        {onStop ? (
          <button
            type="button"
            className="btn btn-install stack-banner-action"
            disabled={busy}
            onClick={onStop}
          >
            Stop
          </button>
        ) : null}
      </div>
    );
  }

  if (state === "starting") {
    return (
      <div
        className="stack-banner stack-banner-starting"
        data-testid="stack-banner"
        data-state="starting"
        role="status"
        aria-live="polite"
      >
        <div className="stack-banner-head">
          <span className="btn-spinner" aria-hidden />
          <strong>Starting stack…</strong>
        </div>
        {phases.length > 0 ? (
          <ul className="stack-phases" data-testid="stack-phases">
            {phases.map((p, i) => {
              const pct = phasePercent(p);
              const last = i === phases.length - 1;
              const done = !last || pct === 100 || p.phase === "done";
              return (
                <li
                  key={p.phase}
                  className={done ? "done" : "active"}
                  data-testid={`stack-phase-${p.phase}`}
                >
                  <span className="stack-phase-mark" aria-hidden>
                    {done ? "✓" : "•"}
                  </span>
                  <span className="stack-phase-name">{p.phase}</span>
                  <span className="muted stack-phase-msg">{p.message}</span>
                  {!done && pct != null ? (
                    <span className="progress">{Math.round(pct)}%</span>
                  ) : null}
                </li>
              );
            })}
          </ul>
        ) : (
          <p className="muted stack-banner-sub">Waiting for services…</p>
        )}
      </div>
    );
  }

  return (
    <div
      className="stack-banner stack-banner-stopped"
      data-testid="stack-banner"
      data-state="stopped"
      role="status"
    >
      <div className="stack-banner-body">
        <strong>Stack stopped, sites are not running</strong>
        <span className="muted">
          Start the web server and database to serve your .test domains.
        </span>
      </div>
      <button
        type="button"
        className="btn btn-primary"
        disabled={busy}
        onClick={onStart}
      >
        Start
      </button>
    </div>
  );
}
