import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const getAppLogs = vi.fn(async () => [
  {
    id: 1,
    ts: "2026-11-08T12:00:00.000Z",
    level: "info" as const,
    source: "app",
    message: "hello",
  },
  {
    id: 2,
    ts: "2026-11-08T12:00:01.000Z",
    level: "error" as const,
    source: "nginx",
    message: "bind failed",
  },
]);
const onAppLog = vi.fn(async () => () => {});
const clearAppLogErrorCount = vi.fn(async () => {});
const clearAppLogs = vi.fn(async () => {});
const openAppLogFolder = vi.fn(async () => "/tmp/logs");

vi.mock("../client", () => ({
  getAppLogs,
  onAppLog,
  clearAppLogErrorCount,
  clearAppLogs,
  openAppLogFolder,
  appendAppLog: vi.fn(),
}));

describe("LogsPage", () => {
  beforeEach(() => {
    getAppLogs.mockClear();
    clearAppLogErrorCount.mockClear();
    // jsdom lacks ResizeObserver
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (globalThis as any).ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    };
  });
  afterEach(() => cleanup());

  it("renders entries and filters by level", async () => {
    const { LogsPage } = await import("./LogsPage");
    render(<LogsPage active />);
    await waitFor(() => expect(screen.getByText("hello")).toBeTruthy());
    expect(screen.getByText("bind failed")).toBeTruthy();
    fireEvent.change(screen.getByTestId("logs-level"), {
      target: { value: "error" },
    });
    expect(screen.queryByText("hello")).toBeNull();
    expect(screen.getByText("bind failed")).toBeTruthy();
    expect(clearAppLogErrorCount).toHaveBeenCalled();
  });
});
