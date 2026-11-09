import { useEffect, useState } from "react";
import * as client from "../client";
import { humanizeError } from "../portBusy";
import type { SiteRecord } from "../types";
import { isValidTld, normalizeTld, parseAliases, tldWarnings } from "../v030";

/** TLD, aliases and wildcard for one site (hosts, certificate and vhost follow). */
export function SiteDomainPanel({
  site,
  onChanged,
}: {
  site: SiteRecord;
  onChanged?: (site: SiteRecord) => void;
}) {
  const currentTld = site.tld ?? site.domain.split(".").slice(-1)[0] ?? "test";
  const [tld, setTld] = useState(currentTld);
  const [aliasText, setAliasText] = useState((site.aliases ?? []).join("\n"));
  const [wildcard, setWildcard] = useState(Boolean(site.wildcard));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notes, setNotes] = useState<string[]>([]);

  useEffect(() => {
    setTld(site.tld ?? site.domain.split(".").slice(-1)[0] ?? "test");
    setAliasText((site.aliases ?? []).join("\n"));
    setWildcard(Boolean(site.wildcard));
  }, [site.id, site.tld, site.domain, site.aliases, site.wildcard]);

  const parsed = parseAliases(aliasText);
  const warnings = tldWarnings(tld);
  const tldOk = isValidTld(tld);

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await client.updateSiteDomain(site.id, {
        tld: normalizeTld(tld) !== currentTld ? normalizeTld(tld) : undefined,
        aliases: parsed.aliases,
        wildcard,
      });
      setNotes([
        ...res.warnings,
        ...(res.hostsOk ? [] : [res.hostsMessage ?? "Hosts file needs repair (Fix hosts)."]),
      ]);
      onChanged?.(res.site);
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="panel-card settings-list" style={{ marginTop: 16 }} data-testid="site-domain-panel">
      <div className="settings-row settings-row-head">
        <label>
          Domain, aliases &amp; TLD
          <span className="hint">{site.domain}</span>
        </label>
        <button
          type="button"
          className="btn btn-primary"
          disabled={busy || !tldOk || parsed.invalid.length > 0}
          onClick={() => void save()}
        >
          {busy ? "Saving…" : "Save"}
        </button>
      </div>
      <div className="settings-row">
        <label>
          TLD
          <span className="hint">`.test` is safest. Changing it renames the site domain.</span>
        </label>
        <input
          aria-label="TLD"
          value={tld}
          style={{ maxWidth: 140 }}
          onChange={(e) => setTld(e.target.value)}
        />
      </div>
      {warnings.length > 0 ? (
        <div className="settings-row settings-row-block" data-testid="tld-warnings">
          {warnings.map((w) => (
            <span key={w} className="error-inline">
              {w}
            </span>
          ))}
        </div>
      ) : null}
      <div className="settings-row settings-row-block">
        <label>
          Aliases
          <span className="hint">One hostname per line, e.g. shop.test</span>
        </label>
        <textarea
          aria-label="Aliases"
          rows={3}
          value={aliasText}
          onChange={(e) => setAliasText(e.target.value)}
          style={{ width: "100%", fontFamily: "var(--mono)", fontSize: 12 }}
        />
        {parsed.invalid.length > 0 ? (
          <span className="error-inline">Invalid: {parsed.invalid.join(", ")}</span>
        ) : null}
      </div>
      <div className="settings-row">
        <label>
          Wildcard sub-domains
          <span className="hint">Also serve *.{site.domain} (certificate + vhost)</span>
        </label>
        <button
          type="button"
          role="switch"
          aria-checked={wildcard}
          aria-label="Wildcard sub-domains"
          className={`toggle ${wildcard ? "on" : ""}`}
          onClick={() => setWildcard(!wildcard)}
        />
      </div>
      {notes.length > 0 ? (
        <div className="settings-row settings-row-block" role="status">
          {notes.map((n) => (
            <span key={n} className="hint">
              {n}
            </span>
          ))}
        </div>
      ) : null}
      {error ? (
        <div className="settings-row">
          <span className="error-inline">{error}</span>
        </div>
      ) : null}
    </div>
  );
}
