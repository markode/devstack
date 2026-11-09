import { useCallback, useEffect, useState } from "react";
import * as client from "../client";
import { humanizeError } from "../portBusy";
import type { DumpStatus, PhpDump } from "../types";

const MAX_DUMPS = 500;

export function formatPayload(payload: unknown): string {
  if (typeof payload === "string") return payload;
  try {
    return JSON.stringify(payload, null, 2) ?? String(payload);
  } catch {
    return String(payload);
  }
}

export function DumpsPage({ active = true }: { active?: boolean }) {
  const [dumps, setDumps] = useState<PhpDump[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<DumpStatus | null>(null);

  const load = useCallback(async () => {
    void client.getDumpStatus().then(setStatus).catch(() => {});
    try {
      setDumps(await client.listPhpDumps(MAX_DUMPS));
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    }
  }, []);

  useEffect(() => {
    if (active) void load();
  }, [active, load]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void client
      .onPhpDump((d) => {
        setDumps((prev) =>
          prev.some((x) => x.id === d.id)
            ? prev
            : [...prev, d].slice(-MAX_DUMPS),
        );
      })
      .then((u) => {
        if (disposed) u();
        else unlisten = u;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const clear = async () => {
    setError(null);
    try {
      await client.clearPhpDumps();
      setDumps([]);
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    }
  };

  const ordered = [...dumps].reverse();

  return (
    <div className="stack" data-testid="dumps-page">
      <div className="section-head dumps-head">
        <div>
          <h2>Dumps</h2>
          <p>
            Values passed to <code>dump()</code> in any site appear here live.
          </p>
        </div>
        <button
          type="button"
          className="btn btn-install"
          disabled={dumps.length === 0}
          onClick={() => void clear()}
        >
          Clear
        </button>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
      {ordered.length === 0 ? (
        <div className="panel-card settings-list" data-testid="dumps-empty">
          <div className="settings-row">
            <label>
              No dumps yet
              <span className="hint">
                Call <code>dump($value)</code> from PHP code running on a
                DevStack site.
                {status?.helperPath ? (
                  <>
                    {" "}
                    Helper: <code>{status.helperPath}</code>
                  </>
                ) : null}
              </span>
            </label>
          </div>
        </div>
      ) : (
        <ul className="dump-list">
          {ordered.map((d) => (
            <li key={d.id} className="panel-card dump-item" data-testid="dump-item">
              <div className="dump-meta muted">
                <span>{new Date(d.ts).toLocaleTimeString()}</span>
                {d.site ? <span>{d.site}</span> : null}
                {d.file ? (
                  <span className="dump-file">
                    {d.file}
                    {d.line ? `:${d.line}` : ""}
                  </span>
                ) : null}
              </div>
              <pre className="code-block">{formatPayload(d.payload)}</pre>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
