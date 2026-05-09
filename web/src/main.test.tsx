import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AppShell } from "./main";

describe("AppShell", () => {
  it("renders the first-screen workbench shell", () => {
    render(<AppShell />);
    expect(screen.getByRole("banner")).toHaveTextContent("Aria Web");
    expect(screen.getByRole("navigation", { name: "Node flow" })).toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveTextContent("Node Workspace");
  });
});
