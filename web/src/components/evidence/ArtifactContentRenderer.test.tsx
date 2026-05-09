import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { ArtifactContentRenderer } from "./ArtifactContentRenderer";

describe("ArtifactContentRenderer", () => {
  it("renders markdown headings with anchor navigation", () => {
    render(<ArtifactContentRenderer contentType="markdown" content={"# Proposal\n\n## Scope\n正文"} />);
    expect(screen.getByRole("link", { name: "Proposal" })).toHaveAttribute("href", "#proposal");
    expect(screen.getByRole("heading", { name: "Scope" })).toHaveAttribute("id", "scope");
  });

  it("folds long json fields by default", async () => {
    render(
      <ArtifactContentRenderer
        contentType="json"
        content={JSON.stringify({
          short: "ok",
          long: "x".repeat(240),
        })}
      />,
    );
    expect(screen.getByText(/long/)).toBeInTheDocument();
    expect(screen.queryByText(/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx/)).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /展开 long/ }));
    expect(screen.getByText(/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx/)).toBeInTheDocument();
  });

  it("renders source test and log content as preformatted text", () => {
    render(<ArtifactContentRenderer contentType="source" content={"export const ok = true;"} />);
    expect(screen.getByText(/export const ok/)).toBeInTheDocument();
  });
});
