import { useEffect, useState } from "react";
import * as client from "../client";
import type { XdebugLaunchHints } from "../types";
import { phpstormSteps, vscodeLaunchSnippet } from "../xdebug";

/** VS Code / PhpStorm launch hints supplied by the backend for one site. */
export function XdebugHints({
  siteId,
  enabled,
}: {
  siteId: string;
  enabled: boolean;
}) {
  const [hints, setHints] = useState<XdebugLaunchHints | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const snippet = hints ? vscodeLaunchSnippet(hints) : "";

  useEffect(() => {
    if (!enabled) {
      setHints(null);
      return;
    }
    let cancelled = false;
    setError(null);
    void client
      .getXdebugLaunchHints(siteId)
      .then((h) => {
        if (!cancelled) setHints(h);
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [siteId, enabled]);

  if (!enabled) return null;
  if (error) {
    return (
      <div className="xdebug-hints muted" data-testid="xdebug-hints">
        IDE hints unavailable: {error}
      </div>
    );
  }
  if (!hints) {
    return (
      <div className="xdebug-hints muted" data-testid="xdebug-hints">
        Loading IDE hints…
      </div>
    );
  }
  return (
    <div className="xdebug-hints" data-testid="xdebug-hints">
      <div className="xdebug-hints-meta muted">
        Listening on port {hints.port} · IDE key <code>{hints.phpstorm.ideKey}</code>
        {hints.launchJson ? (
          <>
            {" "}
            · launch.json: <code>{hints.launchJson}</code>
          </>
        ) : null}
      </div>
      <div className="xdebug-hints-block">
        <div className="xdebug-hints-head">
          <strong>VS Code</strong>
          <button
            type="button"
            className="btn btn-install"
            onClick={() => {
              void navigator.clipboard?.writeText(snippet);
              setCopied(true);
              window.setTimeout(() => setCopied(false), 1500);
            }}
          >
            {copied ? "Copied" : "Copy launch.json"}
          </button>
        </div>
        <pre className="code-block">{snippet}</pre>
      </div>
      <div className="xdebug-hints-block">
        <strong>PhpStorm</strong>
        <ol className="xdebug-steps">
          {phpstormSteps(hints).map((step) => (
            <li key={step}>{step}</li>
          ))}
        </ol>
      </div>
    </div>
  );
}
