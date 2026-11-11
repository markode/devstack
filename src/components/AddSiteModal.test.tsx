import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

vi.mock("../client", () => ({
  listSiteStarterKinds: vi.fn(async () => [
    {
      kind: "empty",
      label: "Empty folder",
      docroot: "",
      commands: ["mkdir"],
      starter: true,
    },
    {
      kind: "php",
      label: "Blank index.php",
      docroot: "",
      commands: ["write index.php"],
      starter: true,
    },
    {
      kind: "wordpress",
      label: "WordPress",
      docroot: "",
      commands: ["wp core install"],
      starter: true,
    },
    {
      kind: "laravel",
      label: "Laravel",
      docroot: "public",
      commands: ["composer create-project"],
      starter: true,
    },
    {
      kind: "react",
      label: "React (Vite)",
      docroot: "dist",
      commands: ["npm create vite"],
      starter: true,
    },
  ]),
  listInstalled: vi.fn(async (id: string) => {
    if (id === "mariadb") {
      return [{ packageId: "mariadb", version: "11.4.0", path: "/db", installedAt: "x" }];
    }
    return [];
  }),
  listPackageVersions: vi.fn(async (id: string) => ({
    packageId: id,
    versions: [
      { version: id === "node" ? "22.11.0" : "8.3.12", channel: "stable", label: "x", lts: true },
    ],
    fromCache: true,
    refreshing: false,
  })),
  installPackage: vi.fn(async (packageId: string, version: string) => ({
    packageId,
    version,
    path: `/${packageId}`,
    installedAt: "now",
  })),
  createSiteFromTemplate: vi.fn(async (req: { kind: string; name: string }) => ({
    site: {
      id: "demo",
      name: req.name,
      path: "~/Sites/demo",
      domain: "demo.test",
      https: true,
    },
    kind: req.kind,
    projectPath: "~/Sites/demo",
    docroot: "~/Sites/demo",
    url: "https://demo.test",
    database: null,
    steps: [{ name: "project", ok: true, skipped: false, message: "ok" }],
    softFailed: false,
    notes: [],
  })),
  cancelSiteTemplateJob: vi.fn(async () => undefined),
  onJobProgress: vi.fn(async () => () => undefined),
  cloneSite: vi.fn(),
}));

vi.mock("../open", () => ({
  browseFolder: vi.fn(async () => null),
}));

import * as client from "../client";
import { AddSiteModal } from "./AddSiteModal";

afterEach(() => {
  cleanup();
  document.body.classList.remove("modal-open");
  vi.clearAllMocks();
});

const phpInstalled = [
  { packageId: "php", version: "8.2.28", path: "/php", installedAt: "x" },
  { packageId: "php", version: "8.1.0", path: "/php", installedAt: "x" },
];
const nodeInstalled = [
  { packageId: "node", version: "20.18.0", path: "/node", installedAt: "x" },
];

describe("AddSiteModal recommended stack", () => {
  it("prefills WordPress with Apache + MariaDB and shows Recommended badges", async () => {
    render(
      <AddSiteModal
        phpInstalled={phpInstalled}
        nodeInstalled={nodeInstalled}
        savedTemplates={[]}
        onClose={() => undefined}
        onCreatedFromTemplate={async () => undefined}
      />,
    );

    fireEvent.click(await screen.findByTestId("starter-wordpress"));

    await waitFor(() =>
      expect((screen.getByTestId("select-webserver") as HTMLSelectElement).value).toBe(
        "apache",
      ),
    );
    expect((screen.getByTestId("select-database") as HTMLSelectElement).value).toBe(
      "mariadb",
    );
    // 8.2 is nearest compatible when 8.3 recommended is missing.
    expect((screen.getByTestId("select-php") as HTMLSelectElement).value).toBe("8.2.28");
    expect(screen.getByTestId("php-fallback-note").textContent).toMatch(/recommended 8\.3/);
    expect(screen.getAllByTestId("recommended-badge").length).toBeGreaterThan(0);
    expect(screen.getByTestId("install-php-recommended")).toBeTruthy();
  });

  it("prefills React with Node and no PHP/DB, warns on low Node", async () => {
    render(
      <AddSiteModal
        phpInstalled={phpInstalled}
        nodeInstalled={[{ packageId: "node", version: "16.20.0", path: "/n", installedAt: "x" }]}
        savedTemplates={[]}
        onClose={() => undefined}
        onCreatedFromTemplate={async () => undefined}
      />,
    );
    fireEvent.click(await screen.findByTestId("starter-react"));
    await waitFor(() => expect(screen.getByTestId("select-node")).toBeTruthy());
    expect(screen.queryByTestId("select-php")).toBeNull();
    expect(screen.queryByTestId("select-database")).toBeNull();
    expect(screen.getByTestId("starter-compat-warning").textContent).toMatch(
      /below the React \(Vite\) minimum/,
    );
  });

  it("creates a Laravel site with the prefilled stack", async () => {
    const onCreated = vi.fn(async () => undefined);
    render(
      <AddSiteModal
        phpInstalled={phpInstalled}
        nodeInstalled={nodeInstalled}
        savedTemplates={[]}
        onClose={() => undefined}
        onCreatedFromTemplate={onCreated}
      />,
    );
    fireEvent.click(await screen.findByTestId("starter-laravel"));
    fireEvent.change(screen.getByTestId("add-site-name"), {
      target: { value: "Shop" },
    });
    fireEvent.click(screen.getByTestId("add-site-submit"));
    await waitFor(() => expect(client.createSiteFromTemplate).toHaveBeenCalled());
    expect(client.createSiteFromTemplate).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "laravel",
        name: "Shop",
        webServer: "nginx",
        databaseEngine: "mariadb",
        phpVersion: "8.2.28",
      }),
    );
    await waitFor(() => expect(onCreated).toHaveBeenCalled());
  });
});
