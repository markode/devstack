import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";

afterEach(() => cleanup());

vi.mock("../client", () => ({
  onJobProgress: vi.fn(async () => () => undefined),
}));

import { AsyncModal, BusyLabel, useAsyncJob } from "./AsyncModal";
import type { JobProgress } from "../types";

describe("AsyncModal shell", () => {
  it("renders progress and disables cancel while busy", () => {
    const progress: JobProgress = {
      jobId: "job",
      phase: "start",
      message: "Starting…",
      bytes: 0,
      total: null,
      percent: null,
    };
    render(
      <AsyncModal
        title="Add site"
        description="Create an empty PHP site"
        busy
        progress={progress}
        error={null}
        onClose={() => undefined}
        actions={
          <>
            <button type="button" data-testid="cancel" disabled>
              Cancel
            </button>
            <button type="button" data-testid="submit" disabled>
              <BusyLabel busy idle="Create site" busyText="Creating…" progress={progress} />
            </button>
          </>
        }
      >
        <input aria-label="Name" defaultValue="demo" />
      </AsyncModal>,
    );
    expect(screen.getByRole("dialog").getAttribute("aria-busy")).toBe("true");
    expect(screen.getAllByText("Starting…").length).toBeGreaterThan(0);
    expect((screen.getByTestId("cancel") as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect(screen.getByLabelText("Name")).toBeTruthy();
    expect(document.querySelector(".modal-job-progress")).toBeTruthy();
  });

  it("shows inline errors in the dialog body", () => {
    render(
      <AsyncModal
        title="Add site"
        busy={false}
        progress={null}
        error="Name is required"
        onClose={() => undefined}
        actions={<button type="button">Create site</button>}
      >
        <span>form</span>
      </AsyncModal>,
    );
    expect(screen.getByText("Name is required")).toBeTruthy();
  });
});

describe("useAsyncJob", () => {
  function Probe({ fail }: { fail?: boolean }) {
    const job = useAsyncJob();
    return (
      <div>
        <button
          type="button"
          data-testid="go"
          onClick={() =>
            void job.run(async () => {
              if (fail) throw new Error("boom");
            })
          }
        >
          Go
        </button>
        {job.error ? <div data-testid="err">{job.error}</div> : null}
      </div>
    );
  }

  it("surfaces thrown errors for inline dialog display", async () => {
    render(<Probe fail />);
    fireEvent.click(screen.getByTestId("go"));
    expect(await screen.findByTestId("err")).toBeTruthy();
    expect(screen.getByTestId("err").textContent).toMatch(/boom/);
  });
});
