import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChoiceRequestEntry } from "./ChoiceRequestEntry";
import { ChoiceResponseEntry } from "./ChoiceResponseEntry";

describe("ChoiceRequestEntry", () => {
  it("submits a single selected option", () => {
    const onRespond = vi.fn();
    const entry = makeChoiceEntry({
      options: [
        { id: "continue", label: "继续" },
        { id: "stop", label: "停止" },
      ],
    });

    render(<ChoiceRequestEntry entry={entry} onRespond={onRespond} />);

    fireEvent.click(screen.getByLabelText("继续"));
    fireEvent.click(screen.getByRole("button", { name: "提交选择" }));

    expect(onRespond).toHaveBeenCalledWith(entry, {
      selected_option_ids: ["continue"],
      free_text: null,
    });
  });

  it("submits multiple selected options", () => {
    const onRespond = vi.fn();
    const entry = makeChoiceEntry({
      allow_multiple: true,
      options: [
        { id: "tests", label: "补测试" },
        { id: "docs", label: "补文档" },
      ],
    });

    render(<ChoiceRequestEntry entry={entry} onRespond={onRespond} />);

    fireEvent.click(screen.getByLabelText("补测试"));
    fireEvent.click(screen.getByLabelText("补文档"));
    fireEvent.click(screen.getByRole("button", { name: "提交选择" }));

    expect(onRespond).toHaveBeenCalledWith(entry, {
      selected_option_ids: ["tests", "docs"],
      free_text: null,
    });
  });

  it("submits free text when free text is allowed", () => {
    const onRespond = vi.fn();
    const entry = makeChoiceEntry({
      allow_free_text: true,
      options: [],
    });

    render(<ChoiceRequestEntry entry={entry} onRespond={onRespond} />);

    fireEvent.change(screen.getByLabelText("补充内容"), {
      target: { value: "请继续实现最小方案" },
    });
    fireEvent.click(screen.getByRole("button", { name: "提交选择" }));

    expect(onRespond).toHaveBeenCalledWith(entry, {
      selected_option_ids: [],
      free_text: "请继续实现最小方案",
    });
  });

  it("hides controls when already resolved", () => {
    const onRespond = vi.fn();
    const entry = makeChoiceEntry({
      resolved: true,
      response: { selected_option_ids: ["continue"], free_text: null },
    });

    render(<ChoiceRequestEntry entry={entry} onRespond={onRespond} />);

    expect(screen.getByText("已选择")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "提交选择" })).not.toBeInTheDocument();
  });

  it("renders choice response entries", () => {
    render(
      <ChoiceResponseEntry
        entry={{
          id: "choice-response-1",
          type: "choice_response",
          role: "user",
          content: "已选择：继续",
          timestamp: "2026-05-26T10:00:00Z",
          metadata: { selected_option_ids: ["continue"] },
        } as ChatEntry}
      />,
    );

    expect(screen.getByText("已选择：继续")).toBeInTheDocument();
  });
});

function makeChoiceEntry(
  metadata: Record<string, unknown> & { resolved?: boolean },
): ChatEntry {
  return {
    id: "choice-request-1",
    type: "choice_request",
    role: "system",
    content: "请选择下一步",
    timestamp: "2026-05-26T10:00:00Z",
    resolved: metadata.resolved === true,
    metadata: {
      request_id: "choice-1",
      prompt: "请选择下一步",
      allow_multiple: false,
      allow_free_text: false,
      ...metadata,
    },
  } as ChatEntry;
}
