import { useEffect, useState, type ReactNode } from "react";
import * as client from "../client";
import { humanizeError } from "../portBusy";
import type { JobProgress } from "../types";

export type AsyncJobState = {
  busy: boolean;
  error: string | null;
  progress: JobProgress | null;
  run: (fn: () => Promise<void>) => Promise<boolean>;
  clearError: () => void;
};

/** Local busy/progress/error for dialogs that run long jobs. */
export function useAsyncJob(): AsyncJobState {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<JobProgress | null>(null);

  useEffect(() => {
    if (!busy) return;
    let unlisten: (() => void) | undefined;
    void client.onJobProgress((ev) => setProgress(ev)).then((u) => {
      unlisten = u;
    });
    return () => unlisten?.();
  }, [busy]);

  const run = async (fn: () => Promise<void>): Promise<boolean> => {
    if (busy) return false;
    setBusy(true);
    setError(null);
    setProgress({
      jobId: "job",
      phase: "start",
      message: "Starting…",
      bytes: 0,
      total: null,
      percent: null,
    });
    try {
      await fn();
      return true;
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
      return false;
    } finally {
      setBusy(false);
      setProgress(null);
    }
  };

  return {
    busy,
    error,
    progress,
    run,
    clearError: () => setError(null),
  };
}

export function ModalProgress({
  progress,
  fallback = "Working…",
}: {
  progress: JobProgress | null;
  fallback?: string;
}) {
  if (!progress) return null;
  const pct =
    progress.percent ??
    (progress.total && progress.total > 0
      ? Math.min(100, (progress.bytes / progress.total) * 100)
      : null);
  return (
    <div className="job-progress modal-job-progress" role="status">
      <div className="job-progress-head">
        <strong>{progress.phase || "Working"}</strong>
        <span className="muted">{progress.message || fallback}</span>
      </div>
      <div className="job-progress-bar">
        <div
          className="job-progress-fill"
          style={{ width: pct != null ? `${pct}%` : "40%" }}
        />
      </div>
    </div>
  );
}

export function BusyLabel({
  busy,
  idle,
  busyText,
  progress,
}: {
  busy: boolean;
  idle: string;
  busyText?: string;
  progress?: JobProgress | null;
}) {
  if (!busy) return <>{idle}</>;
  const label = progress?.message?.trim() || busyText || "Working…";
  return (
    <span className="btn-busy-label">
      <span className="btn-spinner" aria-hidden />
      <span className="btn-busy-text">{label}</span>
    </span>
  );
}

type AsyncModalProps = {
  title: string;
  description?: ReactNode;
  busy: boolean;
  progress?: JobProgress | null;
  error?: string | null;
  onClose: () => void;
  children: ReactNode;
  actions: ReactNode;
  wide?: boolean;
};

/** Shared shell: blocks dismiss while busy, shows inline progress + errors. */
export function AsyncModal({
  title,
  description,
  busy,
  progress,
  error,
  onClose,
  children,
  actions,
  wide,
}: AsyncModalProps) {
  return (
    <div
      className="modal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-busy={busy || undefined}
      onMouseDown={(e) => {
        if (busy) return;
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className="modal-card"
        style={wide ? { maxWidth: 560 } : undefined}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h3>{title}</h3>
        {description ? <p className="muted">{description}</p> : null}
        {busy ? <ModalProgress progress={progress ?? null} /> : null}
        {error ? <div className="error-banner modal-inline-error">{error}</div> : null}
        <fieldset className="modal-fieldset" disabled={busy}>
          {children}
        </fieldset>
        <div className="modal-actions">{actions}</div>
      </div>
    </div>
  );
}
