import { describe, expect, it } from "vitest";
import { siteOpenUrl } from "./open";

describe("siteOpenUrl", () => {
  const stack = {
    httpPort: 8080,
    httpsPort: 8443,
    apacheHttpPort: 8088,
    apacheHttpsPort: 8444,
  };

  it("matches formatSiteUrl for apache http vs https", () => {
    const site = { domain: "test.test", https: false, webServer: "apache" };
    expect(siteOpenUrl(site, stack)).toBe("http://test.test:8088");
    expect(siteOpenUrl({ ...site, https: true }, stack)).toBe(
      "https://test.test:8444",
    );
  });

  it("matches formatSiteUrl for nginx coexistence ports", () => {
    const site = { domain: "blog.test", https: true, webServer: "nginx" };
    expect(siteOpenUrl(site, stack)).toBe("https://blog.test:8443");
    expect(siteOpenUrl({ ...site, https: false }, stack)).toBe(
      "http://blog.test:8080",
    );
  });
});
