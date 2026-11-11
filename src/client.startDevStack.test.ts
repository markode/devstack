import { afterEach, describe, expect, it, vi } from "vitest";

const startMock = vi.fn();

vi.mock("./api", () => ({
  isTauri: () => true,
  startDevStack: () => startMock(),
}));

vi.mock("./mock", () => ({
  mockStackStatus: {
    nginx: false,
    apache: false,
    database: false,
  },
}));

describe("client.startDevStack coalescing", () => {
  afterEach(() => {
    vi.resetModules();
    startMock.mockReset();
    vi.unstubAllEnvs();
  });

  it("shares one promise across concurrent callers", async () => {
    vi.stubEnv("VITE_USE_MOCK", "0");
    let resolveStart!: (v: unknown) => void;
    startMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveStart = resolve as (v: unknown) => void;
        }),
    );
    const client = await import("./client");
    const a = client.startDevStack();
    const b = client.startDevStack();
    expect(startMock).toHaveBeenCalledTimes(1);
    const status = {
      nginx: true,
      apache: true,
      database: true,
      dns: true,
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
    };
    resolveStart?.(status);
    await expect(a).resolves.toEqual(status);
    await expect(b).resolves.toEqual(status);
  });
});
