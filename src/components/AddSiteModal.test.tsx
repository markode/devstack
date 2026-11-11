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
      kind: "html",
      label: "Blank index.html",
      docroot: "",
      commands: ["write index.html"],
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
});

describe("AddSiteModal starters", () => {
  it("lists starters and creates via the template engine", async () => {
    const onCreated = vi.fn(async () => undefined);
    render(
      <AddSiteModal
        phpInstalled={[{ id: "php", name: "PHP", version: "8.3", path: "/php" } as never]}
        nodeInstalled={[{ id: "node", name: "Node", version: "22.0.0", path: "/node" } as never]}
        savedTemplates={[
          {
            id: "tpl1",
            name: "Acme",
            sourceSiteId: "acme",
            sourceDomain: "acme.test",
            sourceUrl: "https://acme.test",
            framework: "wordpress",
            createdAt: "2026-01-01",
            filesArchive: "x.zip",
            sizeBytes: 1,
          },
        ]}
        onClose={() => undefined}
        onCreatedFromTemplate={onCreated}
      />,
    );

    expect(await screen.findByTestId("starter-html")).toBeTruthy();
    expect(screen.getByTestId("starter-wordpress")).toBeTruthy();
    expect(screen.getByTestId("starter-laravel")).toBeTruthy();
    expect(screen.getByTestId("starter-react")).toBeTruthy();
    expect(screen.getByTestId("starter-saved-tpl1")).toBeTruthy();

    fireEvent.click(screen.getByTestId("starter-html"));
    fireEvent.change(screen.getByTestId("add-site-name"), {
      target: { value: "Landing" },
    });
    fireEvent.click(screen.getByTestId("add-site-submit"));

    await waitFor(() => expect(client.createSiteFromTemplate).toHaveBeenCalled());
    expect(client.createSiteFromTemplate).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "html", name: "Landing" }),
    );
    await waitFor(() => expect(onCreated).toHaveBeenCalled());
  });
});
