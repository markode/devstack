import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  SitesStatusBanner,
  bannerState,
  runningSummary,
} from "./SitesStatusBanner";
import type { JobProgress, StackStatus } from "../types";

const stack = {
  nginx: true,
  apache: false,
  database: true,
  databaseEngine: "MariaDB",
} as StackStatus;

const phase = (p: Partial<JobProgress>): JobProgress => ({
  jobId: "stack",
  phase: "nginx",
  message: "",
  bytes: 0,
  total: null,
  percent: null,
  ...p,
});

afterEach(cleanup);

describe("bannerState", () => {
  it("prefers starting over running", () => {
    expect(bannerState(false, true)).toBe("starting");
    expect(bannerState(true, true)).toBe("starting");
    expect(bannerState(true, false)).toBe("running");
    expect(bannerState(false, false)).toBe("stopped");
  });
});

describe("runningSummary", () => {
  it("lists running services", () => {
    expect(runningSummary(stack)).toBe("Running · Nginx · MariaDB");
    expect(runningSummary(null)).toBe("Running");
  });
});

describe("SitesStatusBanner", () => {
  it("shows the large stopped banner with a single Start button", () => {
    const onStart = vi.fn();
    render(
      <SitesStatusBanner
        running={false}
        starting={false}
        phases={[]}
        stack={null}
        onStart={onStart}
      />,
    );
    const banner = screen.getByTestId("stack-banner");
    expect(banner.getAttribute("data-state")).toBe("stopped");
    expect(
      screen.getByText("Stack stopped, sites are not running"),
    ).toBeTruthy();
    const buttons = screen.getAllByRole("button");
    expect(buttons).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    expect(onStart).toHaveBeenCalledTimes(1);
  });

  it("shows per-service progress while starting", () => {
    render(
      <SitesStatusBanner
        running={false}
        starting
        phases={[
          phase({ phase: "database", message: "MariaDB ready", percent: 100 }),
          phase({ phase: "nginx", message: "Binding :80", percent: 40 }),
        ]}
        stack={null}
        onStart={() => {}}
      />,
    );
    expect(screen.getByTestId("stack-banner").getAttribute("data-state")).toBe(
      "starting",
    );
    expect(
      screen.getByTestId("stack-phase-database").className,
    ).toContain("done");
    const active = screen.getByTestId("stack-phase-nginx");
    expect(active.className).toContain("active");
    expect(active.textContent).toContain("Binding :80");
    expect(active.textContent).toContain("40%");
    expect(screen.queryByRole("button", { name: "Start" })).toBeNull();
  });

  it("collapses to a compact green status when running", () => {
    const onStop = vi.fn();
    const { container } = render(
      <SitesStatusBanner
        running
        starting={false}
        phases={[]}
        stack={stack}
        onStart={() => {}}
        onStop={onStop}
      />,
    );
    const banner = screen.getByTestId("stack-banner");
    expect(banner.getAttribute("data-state")).toBe("running");
    expect(banner.className).toContain("stack-banner-compact");
    expect(container.querySelector(".status-dot.on")).not.toBeNull();
    expect(screen.getByText("Running · Nginx · MariaDB")).toBeTruthy();
    expect(
      screen.queryByText("Stack stopped, sites are not running"),
    ).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(onStop).toHaveBeenCalled();
  });
});
