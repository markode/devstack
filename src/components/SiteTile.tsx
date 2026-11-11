import { useState, type KeyboardEvent, type MouseEvent } from "react";
import * as client from "../client";
import { siteOpenUrl } from "../open";
import { formatSiteHost } from "../siteUrl";
import type { SiteDetails, StackStatus } from "../types";

const STATE_LABEL: Record<string, string> = {
  running: "Running",
  stopped: "Stopped",
  error: "Error",
  warning: "Warning",
};

export function siteState(d: SiteDetails): string {
  return d.reachability?.state ?? d.site.status ?? "stopped";
}

/** Match backend reachability copy for empty / incomplete WordPress trees. */
export function isWordpressNotInstalledMessage(message?: string | null): boolean {
  return (message ?? "").trim().toLowerCase() === "wordpress not installed";
}

export function SiteTile({
  details,
  stack,
  phpOptions,
  busy = false,
  onOpenDetail,
  onOpen,
  onOpenUrl,
  onFolder,
  onTerminal,
  onDatabase,
  onHttps,
  onPhp,
  onXdebug,
  onMobilePreview,
}: {
  details: SiteDetails;
  stack: StackStatus | null;
  phpOptions: string[];
  busy?: boolean;
  onOpenDetail: () => void;
  /** Primary Open — parent must use `siteOpenUrl` / `formatSiteUrl`. */
  onOpen: () => void;
  /** Open an arbitrary URL (e.g. wp-admin/install.php). */
  onOpenUrl?: (url: string) => void;
  onFolder: () => void;
  onTerminal: () => void;
  onDatabase: () => void;
  onHttps: (next: boolean) => void;
  onPhp: (version: string | null) => void;
  onXdebug: (next: boolean) => void;
  /** Opens the LAN mobile-preview dialog (button hidden when omitted). */
  onMobilePreview?: () => void;
}) {
  const [logOpen, setLogOpen] = useState(false);
  const [logText, setLogText] = useState<string | null>(null);
  const site = details.site;
  const state = siteState(details);
  const dot =
    state === "running"
      ? "on"
      : state === "error"
        ? "error"
        : state === "warning"
          ? "warn"
          : "";
  const server = (site.webServer ?? "nginx").toLowerCase();
  const url = siteOpenUrl(site, stack);
  const reachMsg = details.reachability?.message ?? null;
  const wpNeedsInstall = isWordpressNotInstalledMessage(reachMsg);
  const hasDb = Boolean(details.databaseName || details.phpmyadminUrl);
  const stop = (e: MouseEvent) => e.stopPropagation();

  const options = [...phpOptions];
  if (site.phpVersion && !options.includes(site.phpVersion)) {
    options.unshift(site.phpVersion);
  }

  const toggleLogs = () => {
    const next = !logOpen;
    setLogOpen(next);
    if (next && !details.reachability?.logExcerpt && logText === null) {
      void client
        .getServiceLog(server, 40)
        .then((t) => setLogText(t || "No log output yet."))
        .catch((e) => setLogText(e instanceof Error ? e.message : String(e)));
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) {
      e.preventDefault();
      onOpenDetail();
    }
  };

  return (
    <article
      className="site-tile"
      data-testid={`site-tile-${site.id}`}
      tabIndex={0}
      onClick={onOpenDetail}
      onKeyDown={onKey}
    >
      <div className="site-tile-top">
        <span className={`fw-icon fw-${details.framework}`} title={details.framework}>
          {details.frameworkIcon}
        </span>
        <div className="site-tile-heading">
          <div className="site-tile-name">
            {site.name}
            {site.isTemplate ? <span className="tpl-badge">template</span> : null}
          </div>
          <div className="site-tile-domain" data-testid="site-tile-url" title={url}>
            {formatSiteHost(site, stack)}
          </div>
        </div>
        <span
          className="site-tile-status"
          title={details.reachability?.message ?? state}
          data-testid="site-tile-status"
        >
          <span className={`status-dot ${dot}`} aria-hidden />
          {STATE_LABEL[state] ?? state}
        </span>
      </div>

      {reachMsg && state !== "running" ? (
        <div
          className={`site-tile-note ${
            state === "error" || state === "warning" ? "bad" : ""
          }`}
          data-testid="site-tile-reachability"
        >
          <span>{reachMsg}</span>
          {wpNeedsInstall && onOpenUrl ? (
            <button
              type="button"
              className="btn btn-install site-tile-note-action"
              data-testid="site-tile-wp-install"
              title="Open WordPress installer"
              onClick={(e) => {
                stop(e);
                onOpenUrl(`${url.replace(/\/$/, "")}/wp-admin/install.php`);
              }}
            >
              Install
            </button>
          ) : null}
        </div>
      ) : null}

      <div className="site-tile-env" onClick={stop}>
        <label className="site-tile-php" title="PHP version">
          <span className="muted">PHP</span>
          {options.length === 0 ? (
            <span>{site.phpVersion ?? "—"}</span>
          ) : (
            <select
              className="native"
              aria-label={`PHP version for ${site.name}`}
              value={site.phpVersion ?? ""}
              disabled={busy}
              onChange={(e) => onPhp(e.target.value || null)}
            >
              <option value="">System</option>
              {options.map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </select>
          )}
        </label>
        <span className="chip" data-testid="site-tile-server">
          {server === "apache" ? "Apache" : "Nginx"}
        </span>
        <button
          type="button"
          className={`chip chip-toggle ${site.xdebug ? "on" : ""}`}
          aria-pressed={Boolean(site.xdebug)}
          title="Toggle Xdebug for this site"
          disabled={busy}
          onClick={() => onXdebug(!site.xdebug)}
        >
          Xdebug
        </button>
        <span className="site-tile-https">
          <span className="muted">HTTPS</span>
          <button
            type="button"
            role="switch"
            aria-checked={site.https}
            aria-label={`HTTPS for ${site.name}`}
            className={`toggle ${site.https ? "on" : ""}`}
            disabled={busy}
            onClick={() => onHttps(!site.https)}
          />
        </span>
      </div>

      <div className="site-tile-actions" onClick={stop}>
        <button type="button" className="btn btn-install" onClick={onOpen}>
          Open
        </button>
        <button type="button" className="btn btn-install" onClick={onFolder}>
          Folder
        </button>
        <button type="button" className="btn btn-install" onClick={onTerminal}>
          Terminal
        </button>
        <button
          type="button"
          className="btn btn-install"
          disabled={!hasDb || busy}
          title={hasDb ? "Open phpMyAdmin" : "No database detected"}
          onClick={onDatabase}
        >
          Database
        </button>
        {onMobilePreview ? (
          <button
            type="button"
            className="btn btn-install"
            title="Preview on a phone over Wi-Fi"
            data-testid="site-tile-mobile"
            onClick={onMobilePreview}
          >
            Mobile
          </button>
        ) : null}
        <button
          type="button"
          className={`btn btn-install ${logOpen ? "active" : ""}`}
          aria-expanded={logOpen}
          onClick={toggleLogs}
        >
          Logs
        </button>
      </div>

      {logOpen ? (
        <pre className="site-tile-log" data-testid="site-tile-log" onClick={stop}>
          {details.reachability?.logExcerpt ?? logText ?? "Loading…"}
        </pre>
      ) : null}
    </article>
  );
}
