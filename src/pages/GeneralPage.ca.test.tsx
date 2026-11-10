import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GeneralPage } from "./GeneralPage";

const { getHostsStatus, repairHosts } = vi.hoisted(() => ({
  getHostsStatus: vi.fn(),
  repairHosts: vi.fn(),
}));

vi.mock("../client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../client")>();
  return {
    ...actual,
    useMock: () => false,
    getAppInfo: vi.fn(async () => ({
      name: "DevStack",
      version: "0.3.1",
      os: "windows",
      arch: "x64",
      dataDir: "C:\\DevStack",
      stack: {},
      developer: { showLogsTab: true, redactSecrets: false },
    })),
    getDbFilesSettings: vi.fn(async () => ({
      folder: null,
      autoExportIntervalSecs: 300,
      autoExportOnStop: true,
      importMode: "ask",
    })),
    getDatabasesFolder: vi.fn(async () => "C:\\DevStack\\Databases"),
    listDatabaseFiles: vi.fn(async () => []),
    getPortsConfig: vi.fn(async () => ({
      coexistenceMode: false,
      httpPort: 80,
      httpsPort: 443,
      apacheHttpPort: 8088,
      apacheHttpsPort: 8444,
      mysqlPort: 3306,
      mariadbPort: 3306,
      postgresqlPort: 5432,
      mongodbPort: 27017,
      phpFpmPort: 9000,
      redisPort: 6379,
      mailpitSmtpPort: 1025,
      mailpitUiPort: 8025,
      dnsPort: 5353,
      memcachedPort: 11211,
      meilisearchPort: 7700,
      typesensePort: 8108,
      minioPort: 9000,
      minioConsolePort: 9001,
      reverbPort: 8080,
    })),
    getDeveloperSettings: vi.fn(async () => ({
      showLogsTab: true,
      redactSecrets: false,
    })),
    getToolsStatus: vi.fn(async () => ({
      composer: true,
      wpCli: true,
      node: true,
      shims: true,
    })),
    getTldSettings: vi.fn(async () => ({
      defaultTld: "test",
      available: ["test", "localhost"],
    })),
    getHostsStatus,
    repairHosts,
  };
});

vi.mock("../components/ParkedDirsSection", () => ({
  ParkedDirsSection: () => null,
}));

vi.mock("../components/ConfigTransferSection", () => ({
  ConfigTransferSection: () => null,
}));

vi.mock("../browse", () => ({
  browseFolder: vi.fn(),
}));

afterEach(cleanup);

describe("GeneralPage CA trust notice", () => {
  beforeEach(() => {
    getHostsStatus.mockReset();
    repairHosts.mockReset();
  });

  it("shows HTTPS certificate not trusted banner with Retry when caTrusted is false", async () => {
    getHostsStatus.mockResolvedValue({
      ok: true,
      missing: [],
      message: null,
      caTrusted: false,
      caMessage:
        "HTTPS certificate not trusted — browsers will warn until the DevStack CA is installed. Click Retry to elevate and trust it.",
    });
    repairHosts.mockResolvedValue({
      ok: true,
      missing: [],
      caTrusted: true,
      caMessage: null,
    });

    render(<GeneralPage />);

    const banner = await screen.findByTestId("ca-untrusted-banner");
    expect(banner.textContent).toMatch(/HTTPS certificate not trusted/i);
    fireEvent.click(screen.getByRole("button", { name: /^Retry$/ }));
    await waitFor(() => expect(repairHosts).toHaveBeenCalledTimes(1));
  });

  it("does not show CA banner when caTrusted is true", async () => {
    getHostsStatus.mockResolvedValue({
      ok: true,
      missing: [],
      caTrusted: true,
      caMessage: null,
    });
    render(<GeneralPage />);
    await screen.findByText("Local DNS / hosts");
    expect(screen.queryByTestId("ca-untrusted-banner")).toBeNull();
  });
});
