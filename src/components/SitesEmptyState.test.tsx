import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SitesEmptyState } from "./SitesEmptyState";

afterEach(cleanup);

describe("SitesEmptyState", () => {
  it("renders the three onboarding steps instead of 'No sites yet' copy", () => {
    render(
      <SitesEmptyState
        onAddFolder={() => {}}
        onCreateWordPress={() => {}}
        onImport={() => {}}
      />,
    );
    expect(screen.getByTestId("sites-empty")).toBeTruthy();
    expect(screen.getByTestId("empty-step-folder")).toBeTruthy();
    expect(screen.getByTestId("empty-step-wordpress")).toBeTruthy();
    expect(screen.getByTestId("empty-step-import")).toBeTruthy();
    expect(screen.queryByText("No sites yet")).toBeNull();
    expect(screen.getByText("Add a folder")).toBeTruthy();
    expect(screen.getAllByText("Create WordPress")).toHaveLength(2);
    expect(screen.getByText("Import a site")).toBeTruthy();
  });

  it("wires each step button", () => {
    const add = vi.fn();
    const wp = vi.fn();
    const imp = vi.fn();
    render(
      <SitesEmptyState
        onAddFolder={add}
        onCreateWordPress={wp}
        onImport={imp}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));
    fireEvent.click(screen.getByRole("button", { name: "Create WordPress" }));
    fireEvent.click(screen.getByRole("button", { name: "Import site" }));
    expect(add).toHaveBeenCalledTimes(1);
    expect(wp).toHaveBeenCalledTimes(1);
    expect(imp).toHaveBeenCalledTimes(1);
  });
});
