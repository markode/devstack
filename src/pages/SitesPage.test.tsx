import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { JobProgress, SiteDetails, StackStatus } from "../types";

const baseStack = {
  nginx: false,
  apache: false,
  dns: true,
  database: false,
  databaseEngine: "mariadb",
  httpPort: 80,
  httpsPort: 443,
  apacheHttpPort: 8088,
  apacheHttpsPort: 8444,
  mysqlPort: 3306,
  mariadbPort: 3306,
  postgresqlPort: 5432,
  mongodbPort: 27017,
  phpFpmPort: 9000,
  coexistenceMode: false,
  phpVersions: [],
} satisfies StackStatus;

let sites: SiteDetails[] = [];
let currentStack: StackStatus = baseStack;
let progressHandler: ((ev: JobProgress) => void) | null = null;
let finishStart: (() => void) | null = null;

vi.mock("../client", () => ({
  listSiteDetails: vi.fn(async () => sites),
  probeAllSiteReachability: vi.fn(async () => sites),
  listInstalled: vi.fn(async () => []),
  getStackStatus: vi.fn(async () => currentStack),
  getTemplates: vi.fn(async () => []),
  getHostsStatus: vi.fn(async () => ({ ok: true, missing: [] })),
  onJobProgress: vi.fn(async (h: (ev: JobProgress) => void) => {
    progressHandler = h;
    return () => {
      progressHandler = null;
    };
  }),
  startDevStack: vi.fn(
    () =>
      new Promise((resolve) => {
        finishStart = () => {
          currentStack = { ...baseStack, nginx: true, database: true };
          resolve(currentStack);
        };
      }),
  ),
  stopDevStack: vi.fn(async () => baseStack),
}));

import { SitesPage } from "./SitesPage";
import { AppDataProvider } from "../store/AppData";

function renderPage() {
  return render(
    <AppDataProvider>
      <SitesPage />
    </AppDataProvider>,
  );
}

beforeEach(() => {
  localStorage.clear();
  sites = [];
  currentStack = baseStack;
  progressHandler = null;
  finishStart = null;
  vi.clearAllMocks();
});
afterEach(cleanup);

describe("SitesPage", () => {
  it("shows onboarding cards when there are no sites", async () => {
    renderPage();
    expect(await screen.findByTestId("sites-empty")).toBeTruthy();
    expect(screen.getByTestId("empty-step-folder")).toBeTruthy();
    expect(screen.getByTestId("empty-step-wordpress")).toBeTruthy();
    expect(screen.getByTestId("empty-step-import")).toBeTruthy();
  });

  it("walks the banner from stopped to starting to running", async () => {
    renderPage();
    const banner = await screen.findByTestId("stack-banner");
    await waitFor(() =>
      expect(screen.getByTestId("stack-banner").getAttribute("data-state")).toBe(
        "stopped",
      ),
    );
    expect(banner).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() =>
      expect(screen.getByTestId("stack-banner").getAttribute("data-state")).toBe(
        "starting",
      ),
    );

    act(() => {
      progressHandler?.({
        jobId: "stack",
        phase: "nginx",
        message: "Binding :80",
        bytes: 0,
        total: null,
        percent: 30,
      });
    });
    expect(await screen.findByTestId("stack-phase-nginx")).toBeTruthy();

    await act(async () => {
      finishStart?.();
    });
    await waitFor(() =>
      expect(screen.getByTestId("stack-banner").getAttribute("data-state")).toBe(
        "running",
      ),
    );
  });

  it("ignores a second Start click while start is in flight", async () => {
    const client = await import("../client");
    renderPage();
    await waitFor(() =>
      expect(screen.getByTestId("stack-banner").getAttribute("data-state")).toBe(
        "stopped",
      ),
    );
    const start = screen.getByRole("button", { name: "Start" });
    fireEvent.click(start);
    fireEvent.click(start);
    fireEvent.click(start);
    await waitFor(() =>
      expect(screen.getByTestId("stack-banner").getAttribute("data-state")).toBe(
        "starting",
      ),
    );
    expect(client.startDevStack).toHaveBeenCalledTimes(1);
    await act(async () => {
      finishStart?.();
    });
    await waitFor(() =>
      expect(screen.getByTestId("stack-banner").getAttribute("data-state")).toBe(
        "running",
      ),
    );
  });

});
