import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatServicePorts,
  isNewService,
  isValidHostname,
  isValidTld,
  levelAtLeast,
  memoryFromStop,
  mergePolledEntries,
  nearestMemoryStop,
  parseAliases,
  pushHistory,
  secondsLabel,
  sizeLabel,
  slidersEqual,
  sortProcesses,
  sparklinePoints,
  splitExcludes,
  syncPlanSummary,
  tldWarnings,
} from "./v030";

describe("services", () => {
  it("knows the new service ids", () => {
    for (const id of ["memcached", "meilisearch", "typesense", "minio", "reverb"]) {
      expect(isNewService(id)).toBe(true);
    }
    expect(isNewService("redis")).toBe(false);
  });

  it("formats MinIO API + console ports", () => {
    expect(formatServicePorts({ id: "minio", ports: [9100, 9101] })).toBe("API :9100 · console :9101");
    expect(formatServicePorts({ id: "memcached", ports: [11211] })).toBe(":11211");
  });
});

describe("formatting", () => {
  it("formats bytes", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(5 * 1024 ** 3)).toBe("5.0 GB");
    expect(formatBytes(-1)).toBe("—");
  });

  it("labels php.ini slider values", () => {
    expect(sizeLabel(null)).toBe("Unlimited");
    expect(sizeLabel(512)).toBe("512 MB");
    expect(sizeLabel(2048)).toBe("2 GB");
    expect(secondsLabel(0)).toBe("Unlimited");
    expect(secondsLabel(120)).toBe("2 min");
    expect(secondsLabel(45)).toBe("45 s");
  });
});

describe("php.ini sliders", () => {
  it("snaps memory to the nearest stop and unlimited", () => {
    expect(memoryFromStop(nearestMemoryStop(500))).toBe(512);
    expect(memoryFromStop(nearestMemoryStop(null))).toBeNull();
    expect(memoryFromStop(-5)).toBe(64);
    expect(memoryFromStop(99)).toBeNull();
  });

  it("compares sliders", () => {
    const a = { memoryLimitMb: 256, maxExecutionTime: 30, uploadMaxMb: 64, postMaxMb: 64 };
    expect(slidersEqual(a, { ...a })).toBe(true);
    expect(slidersEqual(a, { ...a, postMaxMb: 0 })).toBe(false);
  });
});

describe("tld + aliases", () => {
  it("validates hostnames and tlds", () => {
    expect(isValidHostname("shop.test")).toBe(true);
    expect(isValidHostname("nodots")).toBe(false);
    expect(isValidHostname("bad_label.test")).toBe(false);
    expect(isValidTld(".dev")).toBe(true);
    expect(isValidTld("a b")).toBe(false);
  });

  it("parses alias text into unique valid hosts", () => {
    const r = parseAliases("Shop.test, api.shop.test\nshop.test bad_one x.");
    expect(r.aliases).toEqual(["shop.test", "api.shop.test"]);
    expect(r.invalid).toEqual(["bad_one", "x"]);
  });

  it("warns for .dev and .local only", () => {
    expect(tldWarnings("dev")[0]).toContain("HTTPS");
    expect(tldWarnings(".local")[0]).toContain("mDNS");
    expect(tldWarnings("test")).toEqual([]);
  });
});

describe("logs", () => {
  it("ranks levels as minimum severity", () => {
    expect(levelAtLeast("error", "warning")).toBe(true);
    expect(levelAtLeast("info", "warning")).toBe(false);
    expect(levelAtLeast("debug", "all")).toBe(true);
    expect(levelAtLeast("warn", "warning")).toBe(true);
  });

  it("merges polled entries with a cap and reset", () => {
    const e = (m: string) => ({ level: "info", message: m });
    expect(mergePolledEntries([e("a")], [e("b")]).map((x) => x.message)).toEqual(["a", "b"]);
    expect(mergePolledEntries([e("a"), e("b")], [e("c")], 2).map((x) => x.message)).toEqual(["b", "c"]);
    expect(mergePolledEntries([e("a")], [e("z")], 10, true).map((x) => x.message)).toEqual(["z"]);
  });
});

describe("hosting", () => {
  it("summarizes a sync plan", () => {
    expect(syncPlanSummary({ add: 1, update: 2, delete: 0, bytes: 2048 })).toBe("1 new · 2 changed · 2.0 KB");
    expect(syncPlanSummary({ add: 0, update: 0, delete: 0, bytes: 0 })).toBe("Already in sync");
  });

  it("splits exclude text and ignores comments", () => {
    expect(splitExcludes(".git/\n# note\n\n*.log\r\n")).toEqual([".git/", "*.log"]);
  });
});

describe("performance helpers", () => {
  it("keeps a bounded history and draws sparkline points", () => {
    expect(pushHistory([1, 2, 3], 4, 3)).toEqual([2, 3, 4]);
    expect(sparklinePoints([], 100, 20)).toBe("");
    expect(sparklinePoints([0, 10], 100, 20)).toBe("0.0,20.0 100.0,0.0");
  });

  it("sorts running processes by memory", () => {
    const rows = [
      { id: "a", running: false, memoryBytes: 900 },
      { id: "b", running: true, memoryBytes: 10 },
      { id: "c", running: true, memoryBytes: 50 },
    ];
    expect(sortProcesses(rows).map((r) => r.id)).toEqual(["c", "b", "a"]);
  });
});
