import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { useState } from "react";
import type { SiteDetails } from "../types";

const siteA: SiteDetails = {
  site: {
    id: "a",
    name: "alpha",
    path: "~/Sites/alpha",
    domain: "alpha.test",
    https: false,
    status: "running",
  },
  framework: "php",
  frameworkIcon: "PHP",
  absolutePath: "/tmp/alpha",
  url: "http://alpha.test",
  httpsUrl: "https://alpha.test",
  hasWpConfig: false,
};

const siteB: SiteDetails = {
  site: {
    id: "b",
    name: "beta",
    path: "~/Sites/beta",
    domain: "beta.test",
    https: false,
    status: "stopped",
  },
  framework: "wordpress",
  frameworkIcon: "WP",
  absolutePath: "/tmp/beta",
  url: "http://beta.test",
  httpsUrl: "https://beta.test",
  hasWpConfig: true,
};

let listDelayMs = 0;
let probeDelayMs = 80;
let listFail = false;
let listCalls = 0;
let probeCalls = 0;

vi.mock("../client", () => ({
  listSiteDetails: vi.fn(async () => {
    listCalls += 1;
    if (listDelayMs) await new Promise((r) => setTimeout(r, listDelayMs));
    if (listFail) throw new Error("list failed");
    return [siteA, siteB];
  }),
  probeAllSiteReachability: vi.fn(async () => {
    probeCalls += 1;
    if (probeDelayMs) await new Promise((r) => setTimeout(r, probeDelayMs));
    return [
      {
        ...siteA,
        reachability: {
          state: "running",
          message: null,
          tcpOk: true,
          httpOk: true,
          probedUrl: siteA.url,
          probedPort: 80,
        },
      },
      {
        ...siteB,
        reachability: {
          state: "stopped",
          message: null,
          tcpOk: false,
          httpOk: false,
          probedUrl: siteB.url,
          probedPort: 80,
        },
      },
    ];
  }),
  listInstalled: vi.fn(async () => []),
  getStackStatus: vi.fn(async () => ({
    nginx: true,
    apache: false,
    dns: true,
    database: true,
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
  })),
  getTemplates: vi.fn(async () => []),
  getHostsStatus: vi.fn(async () => ({ ok: true, missing: [], message: null })),
  onJobProgress: vi.fn(async () => () => undefined),
  repairHosts: vi.fn(),
  updateSiteEnvironment: vi.fn(),
  checkServicePorts: vi.fn(),
  startDevStack: vi.fn(),
  stopDevStack: vi.fn(),
  setCoexistenceMode: vi.fn(),
  applyNextFreePorts: vi.fn(),
  markTemplate: vi.fn(),
  backupSiteDatabase: vi.fn(),
  restoreSiteDatabase: vi.fn(),
  duplicateSite: vi.fn(),
}));

import { AppDataProvider, useAppData } from "./AppData";
import { SitesPage } from "../pages/SitesPage";

afterEach(() => {
  cleanup();
  listDelayMs = 0;
  probeDelayMs = 80;
  listFail = false;
  listCalls = 0;
  probeCalls = 0;
});

function SitesHarness({
  startOnSites = true,
}: {
  startOnSites?: boolean;
}) {
  const [section, setSection] = useState<"sites" | "php">(
    startOnSites ? "sites" : "php",
  );
  const [visited] = useState(() => new Set<"sites" | "php">(["sites", "php"]));

  return (
    <AppDataProvider>
      <div>
        <button
          type="button"
          data-testid="tab-php"
          onClick={() => setSection("php")}
        >
          PHP
        </button>
        <button
          type="button"
          data-testid="tab-sites"
          onClick={() => setSection("sites")}
        >
          Sites
        </button>
        {visited.has("sites") ? (
          <div hidden={section !== "sites"} data-testid="pane-sites">
            <SitesPage active={section === "sites"} />
          </div>
        ) : null}
        {visited.has("php") ? (
          <div hidden={section !== "php"} data-testid="pane-php">
            <div>PHP page</div>
          </div>
        ) : null}
      </div>
    </AppDataProvider>
  );
}

describe("Sites cache + loading vs empty", () => {
  beforeEach(() => {
    listCalls = 0;
    probeCalls = 0;
  });

  it("shows loading, not empty, before the first successful load", async () => {
    listDelayMs = 60;
    render(<SitesHarness />);
    expect(screen.getByTestId("sites-loading")).toBeTruthy();
    expect(screen.queryByTestId("sites-empty")).toBeNull();
    await waitFor(() => {
      expect(screen.getByTestId("sites-grid")).toBeTruthy();
    });
    expect(screen.queryByTestId("sites-empty")).toBeNull();
    expect(screen.getByText("alpha")).toBeTruthy();
    expect(screen.getByText("beta")).toBeTruthy();
  });

  it("renders cached tiles immediately after tab switch (no empty flash)", async () => {
    render(<SitesHarness />);
    await waitFor(() => expect(screen.getByTestId("sites-grid")).toBeTruthy());
    const callsAfterFirst = listCalls;

    fireEvent.click(screen.getByTestId("tab-php"));
    expect(screen.getByTestId("pane-sites").hasAttribute("hidden")).toBe(true);

    // Simulate a slow refresh that would previously flash empty.
    listDelayMs = 200;
    fireEvent.click(screen.getByTestId("tab-sites"));

    // Cache must paint immediately — not wait for the network.
    expect(screen.getByTestId("sites-grid")).toBeTruthy();
    expect(screen.queryByTestId("sites-empty")).toBeNull();
    expect(screen.getByText("alpha")).toBeTruthy();
    expect(screen.getByText("beta")).toBeTruthy();

    // Background refresh may fire, but must not clear tiles meanwhile.
    await waitFor(() => expect(listCalls).toBeGreaterThan(callsAfterFirst));
    expect(screen.getByTestId("sites-grid")).toBeTruthy();
    expect(screen.queryByTestId("sites-empty")).toBeNull();
  });

  it("keeps cache when a refresh fails", async () => {
    render(<SitesHarness />);
    await waitFor(() => expect(screen.getByTestId("sites-grid")).toBeTruthy());

    listFail = true;
    listDelayMs = 20;
    fireEvent.click(screen.getByTestId("tab-php"));
    fireEvent.click(screen.getByTestId("tab-sites"));

    await waitFor(() => expect(listCalls).toBeGreaterThan(1));
    expect(screen.getByTestId("sites-grid")).toBeTruthy();
    expect(screen.getByText("alpha")).toBeTruthy();
    expect(screen.queryByTestId("sites-empty")).toBeNull();
  });
});

describe("AppData loadSites", () => {
  function Probe() {
    const { sites, loadSites } = useAppData();
    return (
      <div>
        <div data-testid="loaded">{String(sites.loaded)}</div>
        <div data-testid="count">{sites.details.length}</div>
        <div data-testid="refreshing">{String(sites.refreshing)}</div>
        <div data-testid="probing">{String(sites.probing)}</div>
        <button
          type="button"
          data-testid="reload"
          onClick={() => void loadSites({ force: true })}
        >
          reload
        </button>
      </div>
    );
  }

  it("marks loaded after fast list and probes in background", async () => {
    probeDelayMs = 40;
    render(
      <AppDataProvider>
        <Probe />
      </AppDataProvider>,
    );
    await waitFor(() => expect(screen.getByTestId("loaded").textContent).toBe("true"));
    expect(screen.getByTestId("count").textContent).toBe("2");
    await waitFor(() => expect(probeCalls).toBeGreaterThan(0));
    await waitFor(() =>
      expect(screen.getByTestId("probing").textContent).toBe("false"),
    );
  });
});
