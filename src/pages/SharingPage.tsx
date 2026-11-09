import { useCallback, useEffect, useState } from "react";
import * as client from "../client";
import { openInBrowser } from "../open";
import { humanizeError } from "../portBusy";
import type { ShareStatus, SiteRecord } from "../types";

export function SharingPage() {
  const [sites, setSites] = useState<SiteRecord[]>([]);
  const [siteId, setSiteId] = useState("");
  const [status, setStatus] = useState<ShareStatus>({ active: false });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const refresh = useCallback(async () => {
    const [s, st] = await Promise.allSettled([
      client.getSites(),
      client.getShareStatus(),
    ]);
    if (s.status === "fulfilled") {
      setSites(s.value);
      setSiteId((cur) => cur || s.value[0]?.id || "");
    }
    if (st.status === "fulfilled") {
      setStatus(st.value);
      if (st.value.active && st.value.siteId) setSiteId(st.value.siteId);
    } else {
      setError(
        humanizeError(
          st.reason instanceof Error ? st.reason.message : String(st.reason),
        ),
      );
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (fn: () => Promise<ShareStatus>) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await fn());
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
    }
  };

  const copy = () => {
    if (!status.url) return;
    void navigator.clipboard?.writeText(status.url);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  const sharedSite = sites.find((s) => s.id === status.siteId);

  return (
    <div className="stack" data-testid="sharing-page">
      <div className="section-head">
        <h2>Sharing</h2>
        <p>
          Expose one local site through a public tunnel URL. Only the selected
          site is reachable while sharing is on.
        </p>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}

      <div className="panel-card settings-list">
        <div className="settings-row">
          <label>
            Site
            <span className="hint">Pick the site to share</span>
          </label>
          <select
            className="native"
            aria-label="Site to share"
            value={siteId}
            disabled={busy || status.active || sites.length === 0}
            onChange={(e) => setSiteId(e.target.value)}
          >
            {sites.length === 0 ? <option value="">No sites</option> : null}
            {sites.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name} ({s.domain})
              </option>
            ))}
          </select>
        </div>
        <div className="settings-row">
          <label>
            Tunnel
            <span className="hint">
              {status.active
                ? `Sharing ${sharedSite?.domain ?? status.siteId ?? "site"}`
                : "Not sharing"}
            </span>
          </label>
          <div className="row-actions">
            <span className="share-state" data-testid="share-state">
              <span className={`status-dot ${status.active ? "on" : ""}`} aria-hidden />
              {status.active ? "Live" : "Off"}
            </span>
            {status.active ? (
              <button
                type="button"
                className="btn btn-install"
                disabled={busy}
                onClick={() => void run(() => client.stopSiteShare())}
              >
                Stop sharing
              </button>
            ) : (
              <button
                type="button"
                className="btn btn-primary"
                disabled={busy || !siteId}
                onClick={() => void run(() => client.startSiteShare(siteId))}
              >
                {busy ? "Starting…" : "Start sharing"}
              </button>
            )}
          </div>
        </div>
        {status.active && status.url ? (
          <div className="settings-row" data-testid="share-url-row">
            <label>
              Public URL
              <span className="hint">{status.url}</span>
            </label>
            <div className="row-actions">
              <button type="button" className="btn btn-install" onClick={copy}>
                {copied ? "Copied" : "Copy URL"}
              </button>
              <button
                type="button"
                className="btn btn-install"
                onClick={() =>
                  void openInBrowser(status.url!).catch((e) =>
                    setError(e instanceof Error ? e.message : String(e)),
                  )
                }
              >
                Open
              </button>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}
