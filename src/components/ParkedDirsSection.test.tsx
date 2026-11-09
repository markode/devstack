import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const { updateSitesSettings, syncParkedSites } = vi.hoisted(() => ({
  updateSitesSettings: vi.fn(async (p: { parkedDirs?: string[] }) => ({
    parkedDirs: p.parkedDirs ?? [],
    watchEnabled: true,
    optInPath: false,
  })),
  syncParkedSites: vi.fn(async () => ({ added: 0, sites: [] })),
}));

vi.mock("../client", () => ({
  getSitesSettings: vi.fn(async () => ({
    parkedDirs: ["/Users/dev/Sites"],
    watchEnabled: true,
    optInPath: false,
  })),
  updateSitesSettings,
  syncParkedSites,
}));

import { ParkedDirsSection } from "./ParkedDirsSection";

afterEach(cleanup);

describe("ParkedDirsSection", () => {
  it("lists parked dirs, explains name.test, adds and removes", async () => {
    const pick = vi.fn(async () => "/Users/dev/Code");
    render(<ParkedDirsSection pickFolder={pick} />);
    expect(await screen.findByText("/Users/dev/Sites")).toBeTruthy();
    expect(screen.getAllByText("name.test").length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: "Add directory…" }));
    await waitFor(() =>
      expect(updateSitesSettings).toHaveBeenCalledWith({
        parkedDirs: ["/Users/dev/Sites", "/Users/dev/Code"],
      }),
    );
    expect(syncParkedSites).toHaveBeenCalled();

    fireEvent.click(screen.getAllByRole("button", { name: "Remove" })[0]);
    await waitFor(() =>
      expect(updateSitesSettings).toHaveBeenLastCalledWith({
        parkedDirs: ["/Users/dev/Code"],
      }),
    );
  });
});
