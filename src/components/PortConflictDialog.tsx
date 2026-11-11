import { useId } from "react";
import { BusyLabel } from "./AsyncModal";
import { Modal } from "./Modal";
import type { PortCheckResult } from "../types";

type Props = {
  conflict: PortCheckResult;
  busy?: boolean;
  error?: string | null;
  busyLabel?: string;
  onUseSuggested: () => void;
  onCoexistence?: () => void;
  onCancel: () => void;
};

export function PortConflictDialog({
  conflict,
  busy,
  error,
  busyLabel = "Applying…",
  onUseSuggested,
  onCoexistence,
  onCancel,
}: Props) {
  const titleId = useId();
  const suggested = Object.entries(conflict.suggested);
  return (
    <Modal
      onClose={onCancel}
      busy={!!busy}
      labelledBy={titleId}
      style={{ maxWidth: 480 }}
      initialFocus="[data-autofocus]"
    >
      <h3 id={titleId} style={{ marginTop: 0 }}>
        Port already in use
      </h3>
      <p className="muted" style={{ marginTop: 0 }}>
        {conflict.message ??
          `One or more ports for ${conflict.serviceId || "the stack"} are busy.`}
      </p>
      {error ? <div className="error-banner modal-inline-error">{error}</div> : null}
      {busy ? (
        <div className="job-progress modal-job-progress" role="status">
          <div className="job-progress-head">
            <strong>Working</strong>
            <span className="muted">{busyLabel}</span>
          </div>
          <div className="job-progress-bar">
            <div className="job-progress-fill" style={{ width: "40%" }} />
          </div>
        </div>
      ) : null}
      {conflict.conflicts.length ? (
        <ul style={{ fontSize: 13, paddingLeft: 18 }}>
          {conflict.conflicts.map((c) => (
            <li key={c.port}>
              :{c.port}
              {c.processName ? ` — ${c.processName}` : ""}
              {c.hint ? ` (${c.hint})` : ""}
            </li>
          ))}
        </ul>
      ) : null}
      {suggested.length ? (
        <p style={{ fontSize: 13 }}>
          Suggested: {suggested.map(([k, v]) => `${k}=${v}`).join(", ")}
        </p>
      ) : null}
      <div className="modal-actions" style={{ flexWrap: "wrap" }}>
        <button
          type="button"
          className="btn"
          disabled={busy}
          data-autofocus
          title="Keep current ports and cancel"
          onClick={onCancel}
        >
          Cancel
        </button>
        {onCoexistence ? (
          <button
            type="button"
            className="btn btn-install"
            disabled={busy}
            title="Switch to coexistence ports (8080/8443/3307…)"
            onClick={onCoexistence}
          >
            <BusyLabel busy={!!busy} idle="Coexistence mode" busyText={busyLabel} />
          </button>
        ) : null}
        <button
          type="button"
          className="btn btn-primary"
          disabled={busy || !suggested.length}
          title={
            suggested.length
              ? "Apply suggested free ports and retry"
              : "No free-port suggestion available"
          }
          onClick={onUseSuggested}
        >
          <BusyLabel
            busy={!!busy}
            idle={
              suggested.length
                ? `Use suggested port (${suggested.map(([, v]) => v).join(", ")})`
                : "Use suggested port"
            }
            busyText={busyLabel}
          />
        </button>
      </div>
    </Modal>
  );
}
