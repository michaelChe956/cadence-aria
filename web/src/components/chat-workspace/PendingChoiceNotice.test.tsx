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

// F-59（缺陷 2）：等待提示条——修订/生成运行期间 session_state 投影带
// pending choice 时常驻可见，choice 卡未渲染（帧丢失/未送达）也不依赖卡
// 在场；含发问角色、已等待时长与 901s 超时倒计时。
describe("PendingChoiceNotice wait hint (F-59)", () => {
  const waitRequest = (overrides: Record<string, unknown> = {}) => ({
    id: "choice_wait_a",
    prompt: "修订口径需要裁定",
    role: "author",
    created_at_ms: Date.now() - 30_000,
    first_seen_at_ms: Date.now() - 30_000,
    ...overrides,
  });

  it("shows the wait hint from the projection alone, with elapsed time and countdown", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-25T12:00:00Z"));
    try {
      render(
        <PendingChoiceNotice
          entries={[]}
          requests={[waitRequest()]}
          onJump={vi.fn()}
        />,
      );

      const notice = screen.getByTestId("pending-choice-notice");
      expect(notice).toHaveTextContent("⏳ author 有问题等你回答");
      expect(notice).toHaveTextContent("已等待 0:30");
      // 901s 窗口减 30s：剩余 871s = 14:31。
      expect(notice).toHaveTextContent("14:31");
      expect(notice).toHaveTextContent("后超时");
      expect(notice).toHaveTextContent("刷新页面可补卡");
    } finally {
      vi.useRealTimers();
    }
  });

  it("labels reviewer role for reviewer-asked choices", () => {
    render(
      <PendingChoiceNotice
        entries={[]}
        requests={[waitRequest({ role: "reviewer", prompt: "复核口径需要裁定" })]}
        onJump={vi.fn()}
      />,
    );

    expect(screen.getByTestId("pending-choice-notice")).toHaveTextContent(
      "⏳ reviewer 有问题等你回答",
    );
  });

  it("offers the jump button when the card entry exists alongside the projection", () => {
    const onJump = vi.fn();
    render(
      <PendingChoiceNotice
        entries={[
          choiceEntry({ id: "choice-request-wait", prompt: "修订口径需要裁定" }),
        ]}
        requests={[waitRequest({ id: "choice_wait_a" })]}
        onJump={onJump}
      />,
    );

    expect(screen.getByTestId("pending-choice-notice")).toHaveTextContent("⏳ author 有问题等你回答");
    fireEvent.click(screen.getByRole("button", { name: "定位选择卡" }));
    expect(onJump).toHaveBeenCalledWith("choice-request-wait");
  });

  it("omits the countdown when no timestamp is known and falls back to first-seen", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-25T12:00:00Z"));
    try {
      render(
        <PendingChoiceNotice
          entries={[]}
          requests={[
            waitRequest({
              created_at_ms: null,
              first_seen_at_ms: Date.now() - 5_000,
            }),
          ]}
          onJump={vi.fn()}
        />,
      );

      const notice = screen.getByTestId("pending-choice-notice");
      expect(notice).toHaveTextContent("已等待 0:05");
      // 901s 窗口减 5s：剩余 896s = 14:56。
      expect(notice).toHaveTextContent("14:56");
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows the pending count when several choices wait together", () => {
    render(
      <PendingChoiceNotice
        entries={[]}
        requests={[waitRequest(), waitRequest({ id: "choice_wait_b", prompt: "第二问" })]}
        onJump={vi.fn()}
      />,
    );

    expect(screen.getByTestId("pending-choice-notice")).toHaveTextContent("2 个问题");
  });
});
