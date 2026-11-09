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
  return port === defaultPort
    ? `${scheme}://${site.domain}`
    : `${scheme}://${site.domain}:${port}`;
}

/** Short display form (no scheme) for tiles. */
export function formatSiteHost(
  site: UrlSite,
  stack?: Partial<UrlPorts> | null,
): string {
  return formatSiteUrl(site, stack).replace(/^https?:\/\//, "");
}
