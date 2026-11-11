import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Modal } from "./Modal";

afterEach(() => {
  cleanup();
  document.body.classList.remove("modal-open");
});

describe("Modal portal", () => {
  it("mounts under document.body with fixed inset backdrop", async () => {
    const onClose = vi.fn();
    const scrollHost = document.createElement("div");
    scrollHost.className = "content-body";
    document.body.appendChild(scrollHost);
    render(
      <div style={{ height: 1200 }}>
        <Modal onClose={onClose} testId="portal-modal" ariaLabel="Demo">
          <h3>Hello</h3>
          <button type="button" data-autofocus>
            Cancel
          </button>
        </Modal>
      </div>,
      { container: scrollHost },
    );

    const backdrop = await screen.findByTestId("modal-backdrop");
    expect(backdrop.parentElement).toBe(document.body);
    expect(backdrop.classList.contains("modal-backdrop")).toBe(true);
    // Not nested under the scrolled content — escapes transform/overflow traps.
    expect(scrollHost.contains(backdrop)).toBe(false);
    expect(document.body.classList.contains("modal-open")).toBe(true);

    const dialog = screen.getByTestId("portal-modal");
    expect(dialog.getAttribute("role")).toBe("dialog");
    expect(dialog.getAttribute("aria-modal")).toBe("true");

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
    scrollHost.remove();
  });

  it("traps Tab focus inside the dialog", async () => {
    render(
      <Modal onClose={() => undefined} testId="trap-modal" ariaLabel="Trap">
        <button type="button" data-autofocus data-testid="a">
          A
        </button>
        <button type="button" data-testid="b">
          B
        </button>
      </Modal>,
    );
    await waitFor(() => expect(document.activeElement).toBe(screen.getByTestId("a")));
    fireEvent.keyDown(document, { key: "Tab" });
    // Shift from last back to first is asserted via the trap handler on the last element.
    screen.getByTestId("b").focus();
    fireEvent.keyDown(document, { key: "Tab" });
    await waitFor(() => expect(document.activeElement).toBe(screen.getByTestId("a")));
  });
});
