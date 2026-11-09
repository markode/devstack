/** Pure helpers for the v0.3.0 UI (kept dependency-free so they are cheap to test). */

import type {
  ManagedServiceStatus,
  PhpIniSliders,
  SiteLogEntry,
  SyncPlan,
} from "./types";

export const NEW_SERVICE_IDS = [
  "memcached",
  "meilisearch",
  "typesense",
  "minio",
  "reverb",
] as const;

export function isNewService(id: string): boolean {
  return (NEW_SERVICE_IDS as readonly string[]).includes(id);
}

/** `:7700`, or `API :9100 · console :9101` for MinIO. */
export function formatServicePorts(svc: Pick<ManagedServiceStatus, "id" | "ports">): string {
  if (svc.id === "minio" && svc.ports.length >= 2) {
    return `API :${svc.ports[0]} · console :${svc.ports[1]}`;
  }
  return svc.ports.map((p) => `:${p}`).join(" ");
}

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "—";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

export function formatPercent(n: number): string {
  if (!Number.isFinite(n)) return "—";
  return `${n.toFixed(n >= 10 ? 0 : 1)}%`;
}

/** Slider label: `null`/0 → Unlimited, else `512 MB` / `2 GB`. */
export function sizeLabel(mb: number | null): string {
  if (mb == null || mb <= 0) return "Unlimited";
  return mb >= 1024 && mb % 1024 === 0 ? `${mb / 1024} GB` : `${mb} MB`;
}

export function secondsLabel(s: number): string {
  if (s <= 0) return "Unlimited";
  if (s % 60 === 0 && s >= 60) return `${s / 60} min`;
  return `${s} s`;
}

/** Discrete memory slider stops (MB); the last stop means unlimited (-1). */
export const MEMORY_STOPS_MB: (number | null)[] = [64, 128, 256, 512, 1024, 2048, 4096, null];

export function nearestMemoryStop(mb: number | null): number {
  if (mb == null) return MEMORY_STOPS_MB.length - 1;
  let best = 0;
  let diff = Infinity;
  MEMORY_STOPS_MB.forEach((stop, i) => {
    if (stop == null) return;
    const d = Math.abs(stop - mb);
    if (d < diff) {
      diff = d;
      best = i;
    }
  });
  return best;
}

export function memoryFromStop(i: number): number | null {
  return MEMORY_STOPS_MB[Math.max(0, Math.min(MEMORY_STOPS_MB.length - 1, i))];
}

export function slidersEqual(a: PhpIniSliders, b: PhpIniSliders): boolean {
  return (
    a.memoryLimitMb === b.memoryLimitMb &&
    a.maxExecutionTime === b.maxExecutionTime &&
    a.uploadMaxMb === b.uploadMaxMb &&
    a.postMaxMb === b.postMaxMb
  );
}

const HOST_LABEL = /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/;

export function isValidHostname(host: string): boolean {
  const h = host.trim().toLowerCase();
  if (!h.includes(".") || h.length > 253) return false;
  return h.split(".").every((l) => l.length > 0 && l.length <= 63 && HOST_LABEL.test(l));
}

/** Split free text (commas / spaces / newlines) into unique valid aliases. */
export function parseAliases(text: string): { aliases: string[]; invalid: string[] } {
  const aliases: string[] = [];
  const invalid: string[] = [];
  for (const raw of text.split(/[\s,;]+/)) {
    const t = raw.trim().toLowerCase().replace(/\.$/, "");
    if (!t) continue;
    if (isValidHostname(t)) {
      if (!aliases.includes(t)) aliases.push(t);
    } else if (!invalid.includes(t)) {
      invalid.push(t);
    }
  }
  return { aliases, invalid };
}

export function normalizeTld(raw: string): string {
  return raw.trim().replace(/^\.+/, "").toLowerCase();
}

/** Mirrors the backend warnings so the UI can warn before saving. */
export function tldWarnings(tld: string): string[] {
  const t = normalizeTld(tld);
  const out: string[] = [];
  if (["dev", "app", "page", "new", "day"].includes(t)) {
    out.push(`.${t} is HSTS-preloaded: browsers only open it over HTTPS, so HTTPS is forced and the DevStack CA must be trusted.`);
  }
  if (t === "local") {
    out.push(".local is reserved for mDNS (Bonjour/Avahi) — lookups can be slow or bypass the hosts file. Prefer .test.");
  }
  if (["com", "net", "org", "io", "co"].includes(t)) {
    out.push(`.${t} is a real public TLD: the hosts file will shadow the live site on this machine.`);
  }
  return out;
}

export function isValidTld(raw: string): boolean {
  const t = normalizeTld(raw);
  return t.length > 0 && t.length <= 24 && /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(t);
}

const LEVELS = ["debug", "info", "notice", "warning", "error", "critical"];

export function levelRank(level: string): number {
  const l = level.toLowerCase();
  const i = LEVELS.indexOf(l === "warn" ? "warning" : l);
  return i < 0 ? 1 : i;
}

export function levelAtLeast(level: string, min: string): boolean {
  if (!min || min === "all") return true;
  return levelRank(level) >= levelRank(min);
}

/** Append polled entries, drop duplicates at the seam and cap the buffer. */
export function mergePolledEntries(
  existing: SiteLogEntry[],
  incoming: SiteLogEntry[],
  max = 1000,
  reset = false,
): SiteLogEntry[] {
  const base = reset ? [] : existing;
  const merged = [...base, ...incoming];
  return merged.length > max ? merged.slice(merged.length - max) : merged;
}

export function syncPlanSummary(plan: Pick<SyncPlan, "add" | "update" | "delete" | "bytes">): string {
  const parts: string[] = [];
  if (plan.add) parts.push(`${plan.add} new`);
  if (plan.update) parts.push(`${plan.update} changed`);
  if (plan.delete) parts.push(`${plan.delete} deleted`);
  if (!parts.length) return "Already in sync";
  return `${parts.join(" · ")} · ${formatBytes(plan.bytes)}`;
}

export function splitExcludes(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"));
}

/** Rolling history for sparklines. */
export function pushHistory(history: number[], value: number, max = 60): number[] {
  const next = [...history, value];
  return next.length > max ? next.slice(next.length - max) : next;
}

export function sparklinePoints(values: number[], width: number, height: number): string {
  if (values.length === 0) return "";
  const max = Math.max(...values, 1);
  const step = values.length > 1 ? width / (values.length - 1) : 0;
  return values
    .map((v, i) => `${(i * step).toFixed(1)},${(height - (v / max) * height).toFixed(1)}`)
    .join(" ");
}

/** Order managed-process rows: running first, then by memory. */
export function sortProcesses<T extends { running: boolean; memoryBytes: number }>(rows: T[]): T[] {
  return [...rows].sort(
    (a, b) => Number(b.running) - Number(a.running) || b.memoryBytes - a.memoryBytes,
  );
}
