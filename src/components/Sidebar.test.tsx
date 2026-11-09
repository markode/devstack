import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Sidebar } from "./Sidebar";

afterEach(cleanup);

describe("Sidebar", () => {
  it("lists the v0.2.9 sections and routes clicks", () => {
    const onSelect = vi.fn();
    render(<Sidebar active="sites" onSelect={onSelect} />);
    for (const label of ["Sharing", "Mail", "Dumps", "Backups"]) {
      expect(screen.getByRole("button", { name: new RegExp(`${label}$`) })).toBeTruthy();
    }
    fireEvent.click(screen.getByRole("button", { name: /Mail$/ }));
    fireEvent.click(screen.getByRole("button", { name: /Dumps$/ }));
    fireEvent.click(screen.getByRole("button", { name: /Backups$/ }));
    expect(onSelect.mock.calls.map((c) => c[0])).toEqual([
      "mail",
      "dumps",
      "backups",
    ]);
  });

  it("no longer marks Sharing as a placeholder", () => {
    render(<Sidebar active="sharing" onSelect={() => {}} />);
    const sharing = screen.getByRole("button", { name: /Sharing$/ });
    expect(sharing.className).not.toContain("muted");
    expect(sharing.className).toContain("active");
  });
});
