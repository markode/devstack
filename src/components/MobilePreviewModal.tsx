import { useEffect, useId, useState } from "react";
import * as client from "../client";
import { humanizeError } from "../portBusy";
import type { MobilePreviewInfo } from "../types";
import { Modal } from "./Modal";

/** Share a site on the LAN: start/stop preview, QR code, URL + firewall notice. */
export function MobilePreviewModal({
  siteId,
  siteName,
  onClose,
}: {
  siteId: string;
  siteName: string;
  onClose: () => void;
}) {
  const titleId = useId();
  const [info, setInfo] = useState<MobilePreviewInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void client
      .getMobilePreviewStatus()
      .then(setInfo)
      .catch((e) => setError(humanizeError(e instanceof Error ? e.message : String(e))));
  }, []);

  const run = async (fn: () => Promise<MobilePreviewInfo>) => {
    setBusy(true);
    setError(null);
    try {
      setInfo(await fn());
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
    }
  };

  const activeHere = info?.active && info.siteId === siteId;

  return (
    <Modal
      onClose={onClose}
      busy={busy}
      labelledBy={titleId}
      testId="mobile-preview-modal"
      style={{ maxWidth: 460 }}
      initialFocus="[data-autofocus]"
    >
      <h3 id={titleId}>Mobile preview · {siteName}</h3>
      <p className="muted">
        Opens this site to phones and tablets on the same Wi-Fi. The preview URL uses your LAN IP and a
        dedicated port; the site&apos;s hostname is sent as the Host header.
      </p>
      {error ? <div className="error-banner modal-inline-error">{error}</div> : null}
      {info?.active && info.siteId !== siteId ? (
        <div className="port-notice-banner" role="status">
          A preview for another site is running — starting here replaces it.
        </div>
      ) : null}
      {activeHere ? (
        <div style={{ textAlign: "center" }}>
          {info?.qrDataUrl ? (
            <img src={info.qrDataUrl} alt="QR code for the preview URL" width={220} height={220} />
          ) : null}
          <p>
            <code data-testid="mobile-preview-url">{info?.url}</code>
          </p>
          {info?.wpRewritten ? (
            <p className="hint">WordPress URLs are rewritten while the preview runs and restored on stop.</p>
          ) : null}
          {info?.firewallNotice ? <p className="hint">{info.firewallNotice}</p> : null}
          {info?.notes.map((n) => (
            <p key={n} className="hint">
              {n}
            </p>
          ))}
        </div>
      ) : (
        <p className="hint">LAN address: {info?.lanIp ?? "not detected — connect to a network"}</p>
      )}
      <div className="modal-actions">
        <button type="button" className="btn btn-install" disabled={busy} data-autofocus onClick={onClose}>
          Close
        </button>
        {activeHere ? (
          <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void run(client.stopMobilePreview)}>
            Stop preview
          </button>
        ) : (
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || !info?.lanIp}
            onClick={() => void run(() => client.startMobilePreview(siteId))}
          >
            {busy ? "Starting…" : "Start preview"}
          </button>
        )}
      </div>
    </Modal>
  );
}
