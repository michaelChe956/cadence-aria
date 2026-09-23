import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../state/chat-entries";
import { PendingChoiceNotice } from "./PendingChoiceNotice";

function streamEntry(): ChatEntry {
  return {
    id: "entry-1",
    type: "provider_stream",
    role: "author",
    content: "正在输出",
    timestamp: "2026-09-23T04:00:00Z",
  } as ChatEntry;
}

function choiceEntry(overrides: Partial<ChatEntry> & { prompt?: string } = {}): ChatEntry {
  const { prompt = "请选择下一步", ...rest } = overrides;
  return {
    id: "choice-request-1",
    type: "choice_request",
    role: "system",
    content: prompt,
    timestamp: "2026-09-23T04:00:00Z",
    metadata: { request_id: "choice-1", prompt, options: [], allow_multiple: false, allow_free_text: false },
    ...rest,
  } as ChatEntry;
}

describe("PendingChoiceNotice", () => {
  // F-43 ②：卡到达无通知——未处理 choice 出现后用户根本不知道要去应答。
  it("renders nothing while no choice request is pending", () => {
    const { container } = render(
      <PendingChoiceNotice entries={[streamEntry()]} onJump={vi.fn()} />,
    );

    expect(container).toBeEmptyDOMElement();
  });

  it("announces the pending choice and jumps to its card on demand", () => {
    const onJump = vi.fn();

    render(
      <PendingChoiceNotice
        entries={[streamEntry(), choiceEntry({ prompt: "⚠️ Dangerous command:\n\nrm -rf /tmp/x" })]}
        onJump={onJump}
      />,
    );

    const notice = screen.getByTestId("pending-choice-notice");
    expect(notice).toHaveAttribute("role", "status");
    expect(notice).toHaveTextContent("有 1 个选择请求待处理");
    expect(notice).toHaveTextContent("⚠️ Dangerous command:");

    fireEvent.click(screen.getByRole("button", { name: "定位选择卡" }));

    expect(onJump).toHaveBeenCalledWith("choice-request-1");
  });

  it("stops announcing once the choice is resolved", () => {
    const { container } = render(
      <PendingChoiceNotice entries={[choiceEntry({ resolved: true })]} onJump={vi.fn()} />,
    );

    expect(container).toBeEmptyDOMElement();
  });

  it("announces the newest pending choice when several are open", () => {
    const onJump = vi.fn();

    render(
      <PendingChoiceNotice
        entries={[
          choiceEntry({ id: "choice-request-1" }),
          choiceEntry({ id: "choice-request-2", prompt: "第二个问题" }),
        ]}
        onJump={onJump}
      />,
    );

    const notice = screen.getByTestId("pending-choice-notice");
    expect(notice).toHaveTextContent("有 2 个选择请求待处理");
    expect(notice).toHaveTextContent("第二个问题");

    fireEvent.click(screen.getByRole("button", { name: "定位选择卡" }));
    expect(onJump).toHaveBeenCalledWith("choice-request-2");
  });
});
