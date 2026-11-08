import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listPackageVersions = vi.fn();
const listInstalled = vi.fn();
const onInstallProgress = vi.fn(async () => () => {});
const onVersionsUpdated = vi.fn(async () => () => {});

vi.mock("../client", () => ({
  listPackageVersions: listPackageVersions,
  listInstalled: listInstalled,
  onInstallProgress: onInstallProgress,
  onVersionsUpdated: onVersionsUpdated,
  installPackage: vi.fn(),
  uninstallPackage: vi.fn(),
}));

describe("usePackageVersions cache", () => {
  beforeEach(() => {
    vi.resetModules();
    listPackageVersions.mockReset();
    listInstalled.mockReset();
    listPackageVersions.mockResolvedValue({
      packageId: "php",
      versions: [{ version: "8.4.0", label: "8.4.0" }],
      fromCache: true,
      refreshing: false,
      warning: null,
    });
    listInstalled.mockResolvedValue([
      { packageId: "php", version: "8.4.0", path: "/php" },
    ]);
  });

  it("does not show Loading versions after cache is warm (tab switch)", async () => {
    const { usePackageVersions } = await import("./usePackageVersions");

    const first = renderHook(() => usePackageVersions("php"));
    await waitFor(() => expect(first.result.current.loading).toBe(false));
    expect(first.result.current.versions.length).toBeGreaterThan(0);
    first.unmount();

    // Remount as if navigating away and back — must hydrate from memory cache.
    const second = renderHook(() => usePackageVersions("php"));
    expect(second.result.current.loading).toBe(false);
    expect(second.result.current.versions.map((v) => v.version)).toContain(
      "8.4.0",
    );
    // Background refresh may run, but we must never flash empty loading.
    expect(second.result.current.versions.length).toBeGreaterThan(0);
    await act(async () => {
      await Promise.resolve();
    });
    second.unmount();
  });

  it("renders cached disk versions without Loading versions on app restart", async () => {
    // Simulate cold memory + warm disk: first IPC returns fromCache immediately.
    listPackageVersions.mockResolvedValue({
      packageId: "php",
      versions: [
        { version: "8.3.0", label: "8.3.0" },
        { version: "8.4.0", label: "8.4.0" },
      ],
      fromCache: true,
      refreshing: true,
      warning: null,
    });
    const { usePackageVersions } = await import("./usePackageVersions");
    const hook = renderHook(() => usePackageVersions("php"));
    await waitFor(() => expect(hook.result.current.versions.length).toBe(2));
    expect(hook.result.current.loading).toBe(false);
    expect(hook.result.current.fromCache).toBe(true);
    expect(hook.result.current.versions.map((v) => v.version)).toEqual([
      "8.3.0",
      "8.4.0",
    ]);
    hook.unmount();
  });
});
