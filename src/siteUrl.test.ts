import { describe, expect, it } from "vitest";
import { formatSiteHost, formatSiteUrl } from "./siteUrl";

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
});
