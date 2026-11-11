import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

vi.mock("../client", () => ({
  deleteSite: vi.fn(async () => ({ warnings: ["hosts lock skipped"] })),
  onJobProgress: vi.fn(async () => () => undefined),
}));

import * as client from "../client";
import { DeleteSiteModal } from "./DeleteSiteModal";

afterEach(() => {
  cleanup();
  document.body.classList.remove("modal-open");
});

beforeEach(() => {
  vi.clearAllMocks();
});

describe("DeleteSiteModal", () => {
  it("portals into the viewport after the page content is scrolled", async () => {
    const scrollHost = document.createElement("div");
    scrollHost.className = "content-body";
    scrollHost.style.height = "320px";
    scrollHost.style.overflow = "auto";
    const tall = document.createElement("div");
    tall.style.height = "2000px";
    tall.textContent = "spacer";
    scrollHost.appendChild(tall);
    document.body.appendChild(scrollHost);
    scrollHost.scrollTop = 1500;

    const onClose = vi.fn();
    const onDeleted = vi.fn();
    const sitePath = String.raw`C:\Users\Marcin\Sites\acme-dashboard`;
    render(
      <DeleteSiteModal
        siteId="acme"
        siteName="acme-dashboard"
        absolutePath={sitePath}
        databaseName="wp_test"
        databaseUser="u_test"
        onClose={onClose}
        onDeleted={onDeleted}
      />,
    );

    const backdrop = await screen.findByTestId("modal-backdrop");
    expect(backdrop.parentElement).toBe(document.body);
    expect(scrollHost.contains(backdrop)).toBe(false);
    expect(document.body.classList.contains("modal-open")).toBe(true);

    const modal = screen.getByTestId("delete-site-modal");
    expect(modal.getAttribute("role")).toBe("dialog");

    expect(screen.getByText("Remove from DevStack").tagName).toBe("STRONG");
    expect(screen.getByText(/vhosts, hosts entries/i).closest(".modal-check-desc")).toBeTruthy();
    expect(screen.getByText("Drop database and DB user")).toBeTruthy();
    expect(screen.getByText("wp_test / u_test")).toBeTruthy();
    expect(screen.getByText("Delete site folder from disk")).toBeTruthy();
    expect(screen.getByText(sitePath)).toBeTruthy();

    // Title + description are sibling blocks (not one concatenated text node).
    const titles = Array.from(document.querySelectorAll(".modal-check-title")).map(
      (el) => el.textContent,
    );
    expect(titles).toEqual([
      "Remove from DevStack",
      "Drop database and DB user",
      "Delete site folder from disk",
    ]);
    const descs = Array.from(document.querySelectorAll(".modal-check-desc")).map(
      (el) => el.textContent,
    );
    expect(descs[0]).toMatch(/^vhosts,/);
    expect(descs[1]).toBe("wp_test / u_test");
    expect(descs[2]).toBe(sitePath);

    const confirm = screen.getByTestId("delete-site-confirm") as HTMLButtonElement;
    expect(confirm.disabled).toBe(false);
    expect(confirm.className).toMatch(/btn-danger-solid/);

    fireEvent.click(confirm);
    expect(await screen.findByTestId("delete-site-warnings")).toBeTruthy();
    expect(client.deleteSite).toHaveBeenCalledWith("acme", {
      removeFromDevstack: true,
      dropDatabase: false,
      deleteFolder: false,
    });
    fireEvent.click(screen.getByTestId("delete-site-done"));
    await waitFor(() => expect(onDeleted).toHaveBeenCalledWith(["hosts lock skipped"]));

    scrollHost.remove();
  });
});
