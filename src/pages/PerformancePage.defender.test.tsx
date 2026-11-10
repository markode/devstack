import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PerformancePage } from "./PerformancePage";

const { getPerformanceSettings, setDefenderExclusions } = vi.hoisted(() => ({
  getPerformanceSettings: vi.fn(),
  setDefenderExclusions: vi.fn(),
}));

vi.mock("../client", () => ({
  getMetricsSnapshot: vi.fn(async () => ({
    totalCpuPercent: 1,
    totalMemoryBytes: 1024,
    system: {
      cpuPercent: 2,
      cpuCount: 4,
      usedMemoryBytes: 100,
      totalMemoryBytes: 200,
    },
    processes: [],
  })),
  getSlowQueryInfo: vi.fn(async () => ({
    enabled: false,
    running: false,
    engine: null,
    path: null,
    thresholdSecs: 1,
    entries: [],
    notes: [],
  })),
  getSpxStatus: vi.fn(async () => ({
    available: false,
    enabled: false,
    phpVersion: "8.3",
    reportDir: "/tmp",
    uiQuery: "?SPX_KEY=x",
    notes: [],
  })),
  getPerformanceSettings,
  setDefenderExclusions,
  setSlowQueryLog: vi.fn(),
  setSpxEnabled: vi.fn(),
  ensureSpxExtension: vi.fn(),
  enableXdebugProfiling: vi.fn(),
}));

vi.mock("../open", () => ({
  openInFileManager: vi.fn(),
}));

afterEach(cleanup);

describe("PerformancePage Defender exclusions", () => {
  beforeEach(() => {
    getPerformanceSettings.mockReset();
    setDefenderExclusions.mockReset();
  });

  it("shows opt-in toggle default off with UAC explanation on Windows", async () => {
    getPerformanceSettings.mockResolvedValue({
      defenderExclusions: false,
      defenderExclusionsSupported: true,
      exclusionPaths: ["C:\\DevStack"],
      defenderExclusionsHelp:
        "Speeds up PHP/Nginx respawns by skipping Windows Defender scans of the DevStack data folder. This is a system-wide security change (UAC required). Off by default; exclusions are removed when you turn this off or uninstall DevStack.",
    });
    setDefenderExclusions.mockImplementation(async (enabled: boolean) => ({
      defenderExclusions: enabled,
      defenderExclusionsSupported: true,
      exclusionPaths: ["C:\\DevStack"],
      defenderExclusionsHelp: "UAC required. Off by default.",
    }));

    render(<PerformancePage active />);

    const panel = await screen.findByTestId("defender-exclusions-panel");
    expect(panel.textContent).toMatch(/UAC/i);
    expect(panel.textContent).toMatch(/Off by default/i);
    const toggle = screen.getByRole("switch", {
      name: /Windows Defender exclusions/i,
    });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(setDefenderExclusions).toHaveBeenCalledWith(true),
    );
  });

  it("hides the toggle when Defender exclusions are unsupported", async () => {
    getPerformanceSettings.mockResolvedValue({
      defenderExclusions: false,
      defenderExclusionsSupported: false,
      exclusionPaths: [],
      defenderExclusionsHelp: "n/a",
    });
    render(<PerformancePage active />);
    await screen.findByTestId("spx-panel");
    expect(screen.queryByTestId("defender-exclusions-panel")).toBeNull();
  });
});
