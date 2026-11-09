import { useCallback, useEffect, useState } from "react";
import * as client from "../client";
import { openInBrowser } from "../open";
import { humanizeError } from "../portBusy";
import type { ManagedServiceStatus } from "../types";

export function mailpitUrl(
  svc: ManagedServiceStatus | undefined,
  uiPort: number | null,
): string | null {
  if (svc?.uiUrl) return svc.uiUrl;
  if (uiPort) return `http://127.0.0.1:${uiPort}`;
  return null;
}

export function MailpitPage({ active = true }: { active?: boolean }) {
  const [svc, setSvc] = useState<ManagedServiceStatus | undefined>();
  const [uiPort, setUiPort] = useState<number | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const [services, ports] = await Promise.allSettled([
      client.listManagedServices(),
      client.getPortsConfig(),
    ]);
    if (services.status === "fulfilled") {
      setSvc(services.value.find((s) => s.id === "mailpit"));
    } else {
      setError(
        humanizeError(
          services.reason instanceof Error
            ? services.reason.message
            : String(services.reason),
        ),
      );
    }
    if (ports.status === "fulfilled") setUiPort(ports.value.mailpitUi);
    setLoaded(true);
  }, []);

  useEffect(() => {
    if (active) void refresh();
  }, [active, refresh]);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      await client.startManagedService("mailpit");
      await refresh();
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
    }
  };

  const url = mailpitUrl(svc, uiPort);

  return (
    <div className="stack mail-page" data-testid="mail-page">
      <div className="section-head">
        <h2>Mail</h2>
        <p>
          Outgoing PHP mail is caught by Mailpit
          {svc?.ports?.[0] ? ` (SMTP :${svc.ports[0]})` : ""}. Nothing leaves
          this machine.
        </p>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
      {!loaded ? (
        <div className="loading">Loading…</div>
      ) : !svc?.installed ? (
        <div className="panel-card settings-list">
          <div className="settings-row">
            <label>
              Mailpit is not installed
              <span className="hint">Install it from Services.</span>
            </label>
            <button
              type="button"
              className="btn btn-install"
              onClick={() => {
                window.location.hash = "#/services";
              }}
            >
              Go to Services
            </button>
          </div>
        </div>
      ) : !svc.running ? (
        <div className="stack-banner stack-banner-stopped" data-testid="mail-stopped">
          <div className="stack-banner-body">
            <strong>Mailpit is stopped</strong>
            <span className="muted">Start it to read caught messages.</span>
          </div>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy}
            onClick={() => void start()}
          >
            Start
          </button>
        </div>
      ) : url ? (
        <>
          <div className="mail-toolbar">
            <span className="muted">{url}</span>
            <button
              type="button"
              className="btn btn-install"
              onClick={() =>
                void openInBrowser(url).catch((e) =>
                  setError(e instanceof Error ? e.message : String(e)),
                )
              }
            >
              Open in browser
            </button>
          </div>
          <iframe
            className="mail-frame"
            title="Mailpit"
            data-testid="mail-frame"
            src={url}
          />
        </>
      ) : (
        <div className="loading">Mailpit UI URL unavailable.</div>
      )}
    </div>
  );
}
