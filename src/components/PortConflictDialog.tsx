import type { PortCheckResult } from "../types";

type Props = {
  conflict: PortCheckResult;
  busy?: boolean;
  onUseSuggested: () => void;
  onCoexistence?: () => void;
  onCancel: () => void;
};

export function PortConflictDialog({
  conflict,
  busy,
  onUseSuggested,
  onCoexistence,
  onCancel,
}: Props) {
  const suggested = Object.entries(conflict.suggested);
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card" style={{ maxWidth: 480 }}>
        <h3 style={{ marginTop: 0 }}>Port already in use</h3>
        <p className="muted" style={{ marginTop: 0 }}>
          {conflict.message ??
            `One or more ports for ${conflict.serviceId || "the stack"} are busy.`}
        </p>
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
            Suggested:{" "}
            {suggested.map(([k, v]) => `${k}=${v}`).join(", ")}
          </p>
        ) : null}
        <div
          style={{
            display: "flex",
            gap: 8,
            flexWrap: "wrap",
            justifyContent: "flex-end",
            marginTop: 16,
          }}
        >
          <button
            type="button"
            className="btn"
            disabled={busy}
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
              Coexistence mode
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
            Use suggested port
            {suggested.length
              ? ` (${suggested.map(([, v]) => v).join(", ")})`
              : ""}
          </button>
        </div>
      </div>
    </div>
  );
}
