import type { SiteRecord, StackStatus } from "./types";

type UrlSite = Pick<SiteRecord, "domain" | "https" | "webServer">;
type UrlPorts = Pick<
  StackStatus,
  "httpPort" | "httpsPort" | "apacheHttpPort" | "apacheHttpsPort"
>;

const DEFAULT_PORTS: UrlPorts = {
  httpPort: 80,
  httpsPort: 443,
  apacheHttpPort: 8088,
  apacheHttpsPort: 8444,
};

/** Known HTTPS listener ports for a stack (plus standard 443). */
export function stackSslPorts(stack?: Partial<UrlPorts> | null): number[] {
  const ports = { ...DEFAULT_PORTS, ...(stack ?? {}) };
  return Array.from(
    new Set([443, ports.httpsPort, ports.apacheHttpsPort].filter((p) => p > 0)),
  );
}

/**
 * If `url` uses an HTTP scheme on a known SSL port, rewrite to https://.
 */
export function ensureHttpsSchemeForSslPort(
  url: string,
  sslPorts: number[],
): string {
  const trimmed = url.trim();
  const m = trimmed.match(/^(https?):\/\/([^/?#:]+)(?::(\d+))?([/?#].*)?$/i);
  if (!m) return trimmed;
  const scheme = m[1].toLowerCase();
  const host = m[2];
  const portNum = m[3] ? Number(m[3]) : scheme === "https" ? 443 : 80;
  const rest = m[4] ?? "";
  if (scheme === "http" && sslPorts.includes(portNum)) {
    return portNum === 443
      ? `https://${host}${rest}`
      : `https://${host}:${portNum}${rest}`;
  }
  return trimmed;
}

/**
 * Browser URL for a site, including the real listener port (omitted for the
 * scheme default). Port choice depends on the site's web server and scheme.
 */
export function formatSiteUrl(
  site: UrlSite,
  stack?: Partial<UrlPorts> | null,
  opts?: { https?: boolean },
): string {
  const ports = { ...DEFAULT_PORTS, ...(stack ?? {}) };
  const apache = (site.webServer ?? "nginx").toLowerCase() === "apache";
  const https = opts?.https ?? site.https;
  const port = https
    ? apache
      ? ports.apacheHttpsPort
      : ports.httpsPort
    : apache
      ? ports.apacheHttpPort
      : ports.httpPort;
  const scheme = https ? "https" : "http";
  const defaultPort = https ? 443 : 80;
  const raw =
    port === defaultPort
      ? `${scheme}://${site.domain}`
      : `${scheme}://${site.domain}:${port}`;
  return ensureHttpsSchemeForSslPort(raw, stackSslPorts(ports));
}

/** Short display form (no scheme) for tiles. */
export function formatSiteHost(
  site: UrlSite,
  stack?: Partial<UrlPorts> | null,
): string {
  return formatSiteUrl(site, stack).replace(/^https?:\/\//, "");
}
