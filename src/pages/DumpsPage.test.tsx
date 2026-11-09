import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PhpDump } from "../types";

let emit: ((d: PhpDump) => void) | null = null;
const { clearPhpDumps } = vi.hoisted(() => ({
  clearPhpDumps: vi.fn(async () => {}),
}));

vi.mock("../client", () => ({
  listPhpDumps: vi.fn(async () => [
    { id: 1, ts: 1_700_000_000_000, site: "blog.test", payload: { a: 1 }, file: "index.php", line: 7 },
  ]),
  getDumpStatus: vi.fn(async () => ({ port: 9912, helperPath: "/tmp/dump.php" })),
  clearPhpDumps,
  onPhpDump: vi.fn(async (h: (d: PhpDump) => void) => {
    emit = h;
    return () => {};
  }),
}));

import { DumpsPage, formatPayload } from "./DumpsPage";

afterEach(cleanup);

describe("DumpsPage", () => {
  it("formats payloads", () => {
    expect(formatPayload("hi")).toBe("hi");
    expect(formatPayload({ a: 1 })).toBe('{\n  "a": 1\n}');
  });

  it("lists, streams and clears dumps", async () => {
    render(<DumpsPage />);
    expect(await screen.findAllByTestId("dump-item")).toHaveLength(1);
    act(() => {
      emit?.({ id: 2, ts: 1_700_000_001_000, payload: "second" });
    });
    await waitFor(() => expect(screen.getAllByTestId("dump-item")).toHaveLength(2));
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    await waitFor(() => expect(screen.getByTestId("dumps-empty")).toBeTruthy());
    expect(clearPhpDumps).toHaveBeenCalled();
  });
});
