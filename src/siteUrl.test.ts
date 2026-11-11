import { describe, expect, it } from "vitest";
import {
  ensureHttpsSchemeForSslPort,
  formatSiteHost,
  formatSiteUrl,
  stackSslPorts,
} from "./siteUrl";

const site = { domain: "blog.test", https: true, webServer: "nginx" };

describe("formatSiteUrl", () => {
  it("omits default ports", () => {
    expect(formatSiteUrl(site, { httpsPort: 443 })).toBe("https://blog.test");
    expect(
      formatSiteUrl({ ...site, https: false }, { httpPort: 80 }),
    ).toBe("http://blog.test");
  });

  it("includes non-default nginx ports (coexistence)", () => {
    const stack = { httpPort: 8080, httpsPort: 8443 };
    expect(formatSiteUrl(site, stack)).toBe("https://blog.test:8443");
    expect(formatSiteUrl({ ...site, https: false }, stack)).toBe(
      "http://blog.test:8080",
    );
  });

  it("uses apache ports for apache sites", () => {
    const stack = { apacheHttpPort: 8088, apacheHttpsPort: 8444 };
    const apache = { ...site, webServer: "apache" };
    expect(formatSiteUrl(apache, stack)).toBe("https://blog.test:8444");
    expect(formatSiteUrl({ ...apache, https: false }, stack)).toBe(
      "http://blog.test:8088",
    );
  });

  it("falls back to defaults when the stack is unknown", () => {
    expect(formatSiteUrl(site, null)).toBe("https://blog.test");
    expect(
      formatSiteUrl({ ...site, webServer: "apache", https: false }, null),
    ).toBe("http://blog.test:8088");
    expect(formatSiteUrl({ ...site, webServer: null }, undefined)).toBe(
      "https://blog.test",
    );
  });

  it("honours an explicit https override", () => {
    expect(formatSiteUrl(site, { httpPort: 8080 }, { https: false })).toBe(
      "http://blog.test:8080",
    );
  });

  it("formats a scheme-less host", () => {
    expect(formatSiteHost(site, { httpsPort: 8443 })).toBe("blog.test:8443");
  });

  it("forces https when the listen port is an SSL port", () => {
    // Safety net: http + SSL port must never open as plain HTTP.
    expect(
      ensureHttpsSchemeForSslPort("http://blog.test:8443", [443, 8443, 8444]),
    ).toBe("https://blog.test:8443");
    expect(
      ensureHttpsSchemeForSslPort("http://blog.test:443/path", [443, 8443]),
    ).toBe("https://blog.test/path");
    expect(
      ensureHttpsSchemeForSslPort("http://blog.test:8080", [443, 8443]),
    ).toBe("http://blog.test:8080");
    expect(
      ensureHttpsSchemeForSslPort("https://blog.test:8443", [443, 8443]),
    ).toBe("https://blog.test:8443");
  });

  it("exposes stack SSL ports including 443", () => {
    expect(stackSslPorts({ httpsPort: 8443, apacheHttpsPort: 8444 })).toEqual([
      443, 8443, 8444,
    ]);
  });
});
