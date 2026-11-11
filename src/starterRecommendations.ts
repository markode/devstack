import data from "./starterRecommendations.json";
import type { InstalledPackage, PackageVersion, SiteTemplateKind } from "./types";

export type RuntimeReq = {
  recommended: string;
  minimum: string;
  preferLatest?: boolean;
  preferLts?: boolean;
};

export type DatabaseReq = {
  recommended: "mariadb" | "mysql";
  alternatives?: Array<"mariadb" | "mysql">;
  minimumMaria?: string;
  minimumMysql?: string;
};

export type StarterRecommendation = {
  label: string;
  frameworkVersion?: string;
  webServer: "nginx" | "apache";
  php: RuntimeReq | null;
  node: RuntimeReq | null;
  database: DatabaseReq | null;
  notes?: string;
};

type Catalog = {
  updated: string;
  starters: Record<string, StarterRecommendation>;
};

const catalog = data as Catalog;

export function starterRecommendationsUpdated(): string {
  return catalog.updated;
}

export function getStarterRecommendation(
  kind: SiteTemplateKind | string,
): StarterRecommendation | null {
  return catalog.starters[kind] ?? null;
}

/** Parse "8.3", "8.3.12", "v22.11.0" into numeric parts. */
export function parseVersionParts(raw: string): number[] {
  const cleaned = raw.trim().replace(/^v/i, "");
  const core = cleaned.split("-")[0] ?? cleaned;
  return core
    .split(".")
    .map((p) => Number.parseInt(p.replace(/[^\d].*$/, ""), 10))
    .map((n) => (Number.isFinite(n) ? n : 0));
}

export function compareVersions(a: string, b: string): number {
  const pa = parseVersionParts(a);
  const pb = parseVersionParts(b);
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const da = pa[i] ?? 0;
    const db = pb[i] ?? 0;
    if (da !== db) return da < db ? -1 : 1;
  }
  return 0;
}

export function versionMeetsMinimum(version: string, minimum: string): boolean {
  return compareVersions(version, minimum) >= 0;
}

/** Same major.minor family (PHP 8.3.x ↔ 8.3). */
export function sameMinorFamily(version: string, target: string): boolean {
  const a = parseVersionParts(version);
  const b = parseVersionParts(target);
  if (b.length >= 2) return a[0] === b[0] && a[1] === b[1];
  return a[0] === b[0];
}

export type ResolvedRuntime = {
  value: string;
  recommended: string;
  minimum: string;
  status: "exact" | "compatible" | "missing" | "below-minimum";
  note: string | null;
};

export function resolveRuntimeChoice(
  req: RuntimeReq | null | undefined,
  installed: InstalledPackage[],
): ResolvedRuntime | null {
  if (!req) return null;
  const versions = installed
    .map((p) => p.version)
    .sort((a, b) => compareVersions(b, a));

  const exactFamily = versions.filter((v) => sameMinorFamily(v, req.recommended));
  if (exactFamily.length) {
    const best = exactFamily[0];
    return {
      value: best,
      recommended: req.recommended,
      minimum: req.minimum,
      status: sameMinorFamily(best, req.recommended) ? "exact" : "compatible",
      note: null,
    };
  }

  const compatible = versions.filter((v) => versionMeetsMinimum(v, req.minimum));
  if (compatible.length) {
    const best = compatible[0];
    return {
      value: best,
      recommended: req.recommended,
      minimum: req.minimum,
      status: "compatible",
      note: `Using installed ${best} (recommended ${req.recommended} not installed).`,
    };
  }

  if (versions.length) {
    return {
      value: versions[0],
      recommended: req.recommended,
      minimum: req.minimum,
      status: "below-minimum",
      note: `Installed ${versions[0]} is below the minimum ${req.minimum}.`,
    };
  }

  return {
    value: "",
    recommended: req.recommended,
    minimum: req.minimum,
    status: "missing",
    note: `Recommended ${req.recommended} is not installed.`,
  };
}

/** Pick a catalog version to install for a recommendation. */
export function pickInstallVersion(
  req: RuntimeReq,
  available: PackageVersion[],
): string | null {
  if (!available.length) return null;
  const sorted = [...available].sort((a, b) =>
    compareVersions(b.version, a.version),
  );
  const family = sorted.filter((v) => sameMinorFamily(v.version, req.recommended));
  if (family.length) {
    if (req.preferLts) {
      const lts = family.find((v) => v.lts);
      if (lts) return lts.version;
    }
    return family[0].version;
  }
  if (req.preferLts) {
    const lts = sorted.find(
      (v) => v.lts && versionMeetsMinimum(v.version, req.minimum),
    );
    if (lts) return lts.version;
  }
  const ok = sorted.find((v) => versionMeetsMinimum(v.version, req.minimum));
  return ok?.version ?? sorted[0]?.version ?? null;
}

export function incompatibleWarning(
  kind: SiteTemplateKind | string,
  opts: {
    php?: string;
    node?: string;
    webServer?: string;
    database?: string;
  },
): string | null {
  const rec = getStarterRecommendation(kind);
  if (!rec) return null;
  const bits: string[] = [];
  if (rec.php && opts.php && !versionMeetsMinimum(opts.php, rec.php.minimum)) {
    bits.push(
      `PHP ${opts.php} is below the ${rec.label} minimum (${rec.php.minimum}).`,
    );
  }
  if (rec.node && opts.node && !versionMeetsMinimum(opts.node, rec.node.minimum)) {
    bits.push(
      `Node ${opts.node} is below the ${rec.label} minimum (${rec.node.minimum}).`,
    );
  }
  if (
    rec.webServer &&
    opts.webServer &&
    opts.webServer !== rec.webServer &&
    (kind === "wordpress" || kind === "woocommerce") &&
    opts.webServer === "nginx"
  ) {
    bits.push(
      "WordPress pretty permalinks work best on Apache (.htaccess). Nginx needs manual rewrite rules.",
    );
  }
  if (rec.database && opts.database) {
    const allowed = new Set([
      rec.database.recommended,
      ...(rec.database.alternatives ?? []),
    ]);
    if (!allowed.has(opts.database as "mariadb" | "mysql")) {
      bits.push(
        `${opts.database} is not in the recommended engines (${[...allowed].join(", ")}).`,
      );
    }
  }
  return bits.length ? bits.join(" ") : null;
}

export function applyStarterDefaults(kind: SiteTemplateKind | string): {
  webServer: string;
  database: string | null;
  recommendation: StarterRecommendation | null;
} {
  const rec = getStarterRecommendation(kind);
  return {
    webServer: rec?.webServer ?? "nginx",
    database: rec?.database?.recommended ?? null,
    recommendation: rec,
  };
}
