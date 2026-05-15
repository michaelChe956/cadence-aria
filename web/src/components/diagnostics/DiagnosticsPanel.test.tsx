import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { DiagnosticsPanel } from "./DiagnosticsPanel";

describe("DiagnosticsPanel", () => {
  it("keeps unknown diagnostics visible", () => {
    render(<DiagnosticsPanel diagnostics={[{ category: "new_runtime_signal" }]} />);

    expect(screen.getByText("unknown: 1")).toBeInTheDocument();
  });
});
