import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SiteDomainPanel } from "./SiteDomainPanel";
import { SitePhpIniPanel } from "./SitePhpIniPanel";
import { SiteLogsPanel } from "./SiteLogsPanel";
import { Sidebar } from "./Sidebar";
import { SitesEmptyState } from "./SitesEmptyState";
import type { SiteRecord } from "../types";

afterEach(cleanup);

const site: SiteRecord = {
  id: "blog",
  name: "blog",
  path: "~/Sites/blog",
  domain: "blog.test",
  https: true,
  tld: "test",
  aliases: [],
  wildcard: false,
};

describe("v0.3.0 UI", () => {
  it("sidebar lists Performance and Hosting", () => {
    const onSelect = vi.fn();
    render(<Sidebar active="sites" onSelect={onSelect} />);
    fireEvent.click(screen.getByRole("button", { name: /Performance$/ }));
    fireEvent.click(screen.getByRole("button", { name: /Hosting$/ }));
    expect(onSelect.mock.calls.map((c) => c[0])).toEqual(["performance", "hosting"]);
  });

  it("empty state offers templates only when wired", () => {
    const { rerender } = render(
      <SitesEmptyState onAddFolder={() => {}} onCreateWordPress={() => {}} onImport={() => {}} />,
    );
    expect(screen.queryByTestId("empty-step-template")).toBeNull();
    const onTemplates = vi.fn();
    rerender(
      <SitesEmptyState
        onAddFolder={() => {}}
        onCreateWordPress={() => {}}
        onImport={() => {}}
        onTemplates={onTemplates}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Choose template" }));
    expect(onTemplates).toHaveBeenCalled();
  });

  it("php.ini panel loads sliders and applies a change", async () => {
    render(<SitePhpIniPanel siteId="blog" />);
    const memory = (await screen.findByLabelText("memory_limit")) as HTMLInputElement;
    const apply = screen.getByRole("button", { name: "Apply" });
    expect((apply as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(memory, { target: { value: "3" } });
    expect((apply as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(apply);
    await waitFor(() => expect(screen.getByRole("status").textContent).toContain("Saved"));
  });

  it("domain panel warns for .dev and validates aliases", async () => {
    render(<SiteDomainPanel site={site} />);
    fireEvent.change(screen.getByLabelText("TLD"), { target: { value: "dev" } });
    expect(screen.getByTestId("tld-warnings").textContent).toContain("HTTPS");
    fireEvent.change(screen.getByLabelText("Aliases"), { target: { value: "shop.test\nbad_one" } });
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Aliases"), { target: { value: "shop.test" } });
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("logs panel filters entries by level via the backend query", async () => {
    render(<SiteLogsPanel siteId="blog" />);
    await screen.findByText(/Application booted/);
    fireEvent.change(screen.getByLabelText("Level"), { target: { value: "error" } });
    await waitFor(() => expect(screen.queryByText(/Application booted/)).toBeNull());
    expect(screen.getByText(/Undefined variable/)).toBeTruthy();
  });
});
