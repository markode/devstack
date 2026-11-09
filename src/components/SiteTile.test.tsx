import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SiteTile } from "./SiteTile";
import type { SiteDetails, StackStatus } from "../types";

vi.mock("../client", () => ({
  getServiceLog: vi.fn(async () => "nginx: started"),
}));

const stack = {
  httpPort: 8080,
  httpsPort: 8443,
  apacheHttpPort: 8088,
  apacheHttpsPort: 8444,
} as StackStatus;

const details: SiteDetails = {
  site: {
    id: "blog",
    name: "blog",
    path: "~/Sites/blog",
    domain: "blog.test",
    phpVersion: "8.4",
    https: true,
    webServer: "nginx",
    xdebug: false,
    databaseName: "blog",
  },
  framework: "wordpress",
  frameworkIcon: "WP",
  absolutePath: "/Users/dev/Sites/blog",
  url: "http://blog.test",
  httpsUrl: "https://blog.test",
  databaseName: "blog",
  hasWpConfig: true,
  reachability: {
    state: "running",
    tcpOk: true,
    httpOk: true,
    probedUrl: "https://blog.test:8443",
    probedPort: 8443,
  },
};

const handlers = () => ({
  onOpenDetail: vi.fn(),
  onOpenUrl: vi.fn(),
  onFolder: vi.fn(),
  onTerminal: vi.fn(),
  onDatabase: vi.fn(),
  onHttps: vi.fn(),
  onPhp: vi.fn(),
  onXdebug: vi.fn(),
});

afterEach(cleanup);

describe("SiteTile", () => {
  it("shows status, url with real port, php and web server", () => {
    render(
      <SiteTile
        details={details}
        stack={stack}
        phpOptions={["8.5", "8.4"]}
        {...handlers()}
      />,
    );
    expect(screen.getByTestId("site-tile-status").textContent).toContain(
      "Running",
    );
    expect(screen.getByTestId("site-tile-url").textContent).toBe(
      "blog.test:8443",
    );
    expect(screen.getByTestId("site-tile-server").textContent).toBe("Nginx");
    const php = screen.getByLabelText("PHP version for blog") as HTMLSelectElement;
    expect(php.value).toBe("8.4");
  });

  it("fires quick actions without opening the detail view", () => {
    const h = handlers();
    render(
      <SiteTile details={details} stack={stack} phpOptions={["8.4"]} {...h} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(h.onOpenUrl).toHaveBeenCalledWith("https://blog.test:8443");
    fireEvent.click(screen.getByRole("button", { name: "Folder" }));
    fireEvent.click(screen.getByRole("button", { name: "Terminal" }));
    fireEvent.click(screen.getByRole("button", { name: "Database" }));
    fireEvent.click(screen.getByRole("switch", { name: "HTTPS for blog" }));
    fireEvent.click(screen.getByRole("button", { name: "Xdebug" }));
    expect(h.onFolder).toHaveBeenCalled();
    expect(h.onTerminal).toHaveBeenCalled();
    expect(h.onDatabase).toHaveBeenCalled();
    expect(h.onHttps).toHaveBeenCalledWith(false);
    expect(h.onXdebug).toHaveBeenCalledWith(true);
    expect(h.onOpenDetail).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("site-tile-blog"));
    expect(h.onOpenDetail).toHaveBeenCalledTimes(1);
  });

  it("expands logs", async () => {
    render(
      <SiteTile
        details={details}
        stack={stack}
        phpOptions={["8.4"]}
        {...handlers()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Logs" }));
    expect(await screen.findByText("nginx: started")).toBeTruthy();
  });
});
