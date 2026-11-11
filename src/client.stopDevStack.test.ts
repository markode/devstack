import { afterEach, describe, expect, it, vi } from "vitest";

const stopMock = vi.fn();

vi.mock("./api", () => ({
  isTauri: () => true,
  stopDevStack: () => stopMock(),
}));

vi.mock("./mock", () => ({
  mockStackStatus: {
    nginx: true,
    apache: true,
    database: true,
  },
}));

describe("client.stopDevStack coalescing", () => {
  afterEach(() => {
    vi.resetModules();
    stopMock.mockReset();
    vi.unstubAllEnvs();
  });

  it("shares one promise across concurrent callers", async () => {
    vi.stubEnv("VITE_USE_MOCK", "0");
    let resolveStop!: (v: unknown) => void;
    stopMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveStop = resolve as (v: unknown) => void;
        }),
    );
    const client = await import("./client");
    const a = client.stopDevStack();
    const b = client.stopDevStack();
    expect(stopMock).toHaveBeenCalledTimes(1);
    const status = {
      nginx: false,
      apache: false,
      database: false,
      dns: false,
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
    resolveStop?.(status);
    await expect(a).resolves.toEqual(status);
    await expect(b).resolves.toEqual(status);
  });
});
