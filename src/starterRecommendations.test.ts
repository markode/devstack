import { describe, expect, it } from "vitest";
import {
  applyStarterDefaults,
  compareVersions,
  getStarterRecommendation,
  incompatibleWarning,
  pickInstallVersion,
  resolveRuntimeChoice,
  sameMinorFamily,
  versionMeetsMinimum,
} from "./starterRecommendations";

describe("starterRecommendations catalog", () => {
  it("recommends Apache + PHP + MariaDB for WordPress / Woo", () => {
    for (const kind of ["wordpress", "woocommerce"] as const) {
      const rec = getStarterRecommendation(kind);
      expect(rec?.webServer).toBe("apache");
      expect(rec?.php?.minimum).toBe("8.1");
      expect(rec?.database?.recommended).toBe("mariadb");
      expect(rec?.node).toBeNull();
    }
  });

  it("recommends Nginx + PHP 8.2+ + DB for Laravel", () => {
    const rec = getStarterRecommendation("laravel");
    expect(rec?.webServer).toBe("nginx");
    expect(rec?.php?.minimum).toBe("8.2");
    expect(rec?.database?.recommended).toBe("mariadb");
    expect(applyStarterDefaults("laravel").webServer).toBe("nginx");
  });

  it("recommends Node LTS and no PHP/DB for JS frameworks", () => {
    for (const kind of ["nextjs", "react", "vue", "angular"] as const) {
      const rec = getStarterRecommendation(kind);
      expect(rec?.php).toBeNull();
      expect(rec?.database).toBeNull();
      expect(rec?.node?.recommended).toBe("22");
      expect(rec?.webServer).toBe("nginx");
    }
  });

  it("recommends Nginx + latest PHP for blank PHP", () => {
    const rec = getStarterRecommendation("php");
    expect(rec?.webServer).toBe("nginx");
    expect(rec?.php?.preferLatest).toBe(true);
    expect(rec?.database).toBeNull();
  });
});

describe("version helpers", () => {
  it("compares and matches minors", () => {
    expect(compareVersions("8.3.12", "8.3")).toBeGreaterThan(0);
    expect(versionMeetsMinimum("8.2.0", "8.2")).toBe(true);
    expect(versionMeetsMinimum("8.1.9", "8.2")).toBe(false);
    expect(sameMinorFamily("8.3.12", "8.3")).toBe(true);
    expect(sameMinorFamily("8.4.0", "8.3")).toBe(false);
  });

  it("falls back to nearest installed compatible runtime", () => {
    const installed = [
      {
        packageId: "php",
        version: "8.2.28",
        path: "/php",
        installedAt: "x",
      },
      {
        packageId: "php",
        version: "8.1.0",
        path: "/php",
        installedAt: "x",
      },
    ];
    const resolved = resolveRuntimeChoice(
      { recommended: "8.4", minimum: "8.2", preferLatest: true },
      installed,
    );
    expect(resolved?.status).toBe("compatible");
    expect(resolved?.value).toBe("8.2.28");
    expect(resolved?.note).toMatch(/recommended 8\.4/);
  });

  it("picks an installable catalog version for the recommended family", () => {
    const ver = pickInstallVersion(
      { recommended: "22", minimum: "18.18", preferLts: true },
      [
        { version: "23.0.0", channel: "current", label: "23", lts: false },
        { version: "22.11.0", channel: "lts", label: "22 LTS", lts: true },
        { version: "20.18.0", channel: "lts", label: "20 LTS", lts: true },
      ],
    );
    expect(ver).toBe("22.11.0");
  });

  it("warns when PHP is below the framework minimum", () => {
    const w = incompatibleWarning("laravel", {
      php: "8.1.0",
      webServer: "nginx",
      database: "mariadb",
    });
    expect(w).toMatch(/below the Laravel minimum/);
  });
});
