import { useCallback, useEffect, useState } from "react";
import * as client from "../client";
import { humanizeError } from "../portBusy";
import type { PhpIniSliders, SitePhpIniView } from "../types";
import {
  MEMORY_STOPS_MB,
  memoryFromStop,
  nearestMemoryStop,
  secondsLabel,
  sizeLabel,
  slidersEqual,
} from "../v030";

/** Per-site php.ini overrides (sliders + advanced keys) merged over the global ini. */
export function SitePhpIniPanel({ siteId }: { siteId: string }) {
  const [view, setView] = useState<SitePhpIniView | null>(null);
  const [draft, setDraft] = useState<PhpIniSliders | null>(null);
  const [extra, setExtra] = useState<Record<string, string>>({});
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const apply = useCallback((v: SitePhpIniView) => {
    setView(v);
    setDraft(v.sliders);
    const slider = new Set([
      "memory_limit",
      "max_execution_time",
      "upload_max_filesize",
      "post_max_size",
    ]);
    setExtra(
      Object.fromEntries(Object.entries(v.overrides).filter(([k]) => !slider.has(k))),
    );
  }, []);

  useEffect(() => {
    let alive = true;
    void client
      .getSitePhpIni(siteId)
      .then((v) => alive && apply(v))
      .catch((e) => alive && setError(humanizeError(e instanceof Error ? e.message : String(e))));
    return () => {
      alive = false;
    };
  }, [siteId, apply]);

  const run = async (fn: () => Promise<SitePhpIniView>, done: string) => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const v = await fn();
      apply(v);
      setMessage(
        v.notes.length ? `${done} · ${v.notes.join(" ")}` : v.reloaded ? `${done} · PHP reloaded` : done,
      );
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
    }
  };

  if (!view || !draft) {
    return (
      <div className="panel-card settings-list" style={{ marginTop: 16 }} data-testid="site-php-ini">
        <div className="settings-row settings-row-head">
          <label>PHP settings (this site)</label>
        </div>
        <div className="settings-row">
          {error ? <span className="error-inline">{error}</span> : <span className="muted">Loading…</span>}
        </div>
      </div>
    );
  }

  const dirty = !slidersEqual(draft, view.sliders);
  const hasOverrides = Object.keys(view.overrides).length > 0;
  const known = view.knownKeys.filter(
    (k) => !["memory_limit", "max_execution_time", "upload_max_filesize", "post_max_size"].includes(k.key),
  );

  return (
    <div className="panel-card settings-list" style={{ marginTop: 16 }} data-testid="site-php-ini">
      <div className="settings-row settings-row-head">
        <label>
          PHP settings (this site)
          <span className="hint">
            Overrides merge on top of the global php.ini{view.phpVersion ? ` · PHP ${view.phpVersion}` : ""}
          </span>
        </label>
        <div className="row-actions">
          {hasOverrides ? (
            <button
              type="button"
              className="btn btn-install"
              disabled={busy}
              onClick={() => void run(() => client.setSitePhpIni(siteId, {}), "Reset to global")}
            >
              Reset
            </button>
          ) : null}
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || !dirty}
            onClick={() => void run(() => client.setSitePhpIniSliders(siteId, draft), "Saved")}
          >
            {busy ? "Applying…" : "Apply"}
          </button>
        </div>
      </div>

      <div className="settings-row">
        <label>
          memory_limit
          <span className="hint">{sizeLabel(draft.memoryLimitMb)}</span>
        </label>
        <input
          type="range"
          aria-label="memory_limit"
          min={0}
          max={MEMORY_STOPS_MB.length - 1}
          step={1}
          value={nearestMemoryStop(draft.memoryLimitMb)}
          onChange={(e) => setDraft({ ...draft, memoryLimitMb: memoryFromStop(Number(e.target.value)) })}
        />
      </div>
      <div className="settings-row">
        <label>
          max_execution_time
          <span className="hint">{secondsLabel(draft.maxExecutionTime)}</span>
        </label>
        <input
          type="range"
          aria-label="max_execution_time"
          min={0}
          max={600}
          step={5}
          value={draft.maxExecutionTime}
          onChange={(e) => setDraft({ ...draft, maxExecutionTime: Number(e.target.value) })}
        />
      </div>
      <div className="settings-row">
        <label>
          upload_max_filesize
          <span className="hint">{sizeLabel(draft.uploadMaxMb)}</span>
        </label>
        <input
          type="range"
          aria-label="upload_max_filesize"
          min={0}
          max={2048}
          step={16}
          value={draft.uploadMaxMb}
          onChange={(e) => setDraft({ ...draft, uploadMaxMb: Number(e.target.value) })}
        />
      </div>
      <div className="settings-row">
        <label>
          post_max_size
          <span className="hint">{sizeLabel(draft.postMaxMb)}</span>
        </label>
        <input
          type="range"
          aria-label="post_max_size"
          min={0}
          max={2048}
          step={16}
          value={draft.postMaxMb}
          onChange={(e) => setDraft({ ...draft, postMaxMb: Number(e.target.value) })}
        />
      </div>

      <div className="settings-row settings-row-block">
        <label>Advanced keys</label>
        {Object.entries(extra).map(([k, v]) => (
          <div key={k} className="field-row" style={{ marginTop: 6 }}>
            <code style={{ flex: "0 0 200px" }}>{k}</code>
            <input
              value={v}
              aria-label={k}
              onChange={(e) => setExtra({ ...extra, [k]: e.target.value })}
            />
            <button
              type="button"
              className="btn btn-install"
              disabled={busy}
              onClick={() => {
                const { [k]: _drop, ...rest } = extra;
                setExtra(rest);
              }}
            >
              Remove
            </button>
          </div>
        ))}
        <div className="field-row" style={{ marginTop: 8 }}>
          <select className="native" value={newKey} onChange={(e) => setNewKey(e.target.value)} aria-label="php.ini key">
            <option value="">Add key…</option>
            {known
              .filter((k) => !(k.key in extra))
              .map((k) => (
                <option key={k.key} value={k.key}>
                  {k.key} — {k.description}
                </option>
              ))}
          </select>
          <input
            placeholder="value"
            value={newValue}
            aria-label="php.ini value"
            onChange={(e) => setNewValue(e.target.value)}
          />
          <button
            type="button"
            className="btn btn-install"
            disabled={!newKey || !newValue.trim()}
            onClick={() => {
              setExtra({ ...extra, [newKey]: newValue.trim() });
              setNewKey("");
              setNewValue("");
            }}
          >
            Add
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy}
            onClick={() =>
              void run(
                () =>
                  client.setSitePhpIni(siteId, {
                    ...view.overrides,
                    ...extra,
                  }),
                "Saved advanced keys",
              )
            }
          >
            Save keys
          </button>
        </div>
        {message ? <span className="hint" role="status">{message}</span> : null}
        {error ? <span className="error-inline">{error}</span> : null}
        <span className="hint">Override file: {view.iniPath}</span>
      </div>
    </div>
  );
}
