import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ProviderExecutionPanel } from "./ProviderExecutionPanel";

describe("ProviderExecutionPanel", () => {
  it("renders provider input refs summaries and output chunks without full prompts", () => {
    render(
      <ProviderExecutionPanel
        events={[
          {
            cursor: 1,
            event_type: "provider_input",
            task_id: "task_0001",
            payload: {
              node_id: "N16",
              provider: "codex",
              input_ref: "input://task_0001/N16",
              input_summary: "需要实现项目选择器",
              input_full: "SECRET_INPUT_FULL_SHOULD_NOT_RENDER",
              prompt: "SECRET_FULL_PROMPT_SHOULD_NOT_RENDER",
            },
          },
          {
            cursor: 2,
            event_type: "provider_output",
            task_id: "task_0001",
            payload: {
              node_id: "N16",
              provider_run_id: "run_0001",
              stream: "stdout",
              text: "chunk: created picker",
              input_ref: "input://task_0001/N16",
            },
          },
        ]}
      />,
    );

    const panel = screen.getByRole("region", { name: "Provider execution panel" });
    expect(within(panel).getByText("input://task_0001/N16")).toBeInTheDocument();
    expect(within(panel).getByText("需要实现项目选择器")).toBeInTheDocument();
    expect(within(panel).getByText("chunk: created picker")).toBeInTheDocument();
    expect(panel).not.toHaveTextContent("SECRET_INPUT_FULL_SHOULD_NOT_RENDER");
    expect(panel).not.toHaveTextContent("SECRET_FULL_PROMPT_SHOULD_NOT_RENDER");
  });
});
