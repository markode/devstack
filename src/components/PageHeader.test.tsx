import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PageHeader } from "./PageHeader";

describe("PageHeader", () => {
  it("renders Herd-style centered title with optional meta and loader", () => {
    render(
      <PageHeader title="Sites" meta="2 sites" refreshing />,
    );
    expect(screen.getByTestId("page-header-title").textContent).toBe("Sites");
    expect(screen.getByText("2 sites")).toBeTruthy();
    expect(screen.getByTestId("sites-header-loader")).toBeTruthy();
  });
});
