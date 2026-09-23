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

  it("renders and submits all questions in one structured choice request", () => {
    const onRespond = vi.fn();
    const entry = makeChoiceEntry({
      prompt: "请确认 3 个关键点",
      questions: [
        {
          id: "startup",
          prompt: "启动自检策略？",
          allow_multiple: false,
          allow_free_text: false,
          options: [
            { id: "self_check", label: "每次启动都自检" },
            { id: "failure_only", label: "仅失败后自检" },
          ],
        },
        {
          id: "scope",
          prompt: "影响范围？",
          allow_multiple: false,
          allow_free_text: false,
          options: [
            { id: "story_only", label: "仅 Story Spec" },
            { id: "shared", label: "Story/Design/Work Item 共享链路" },
          ],
        },
        {
          id: "mcp_events",
          prompt: "MCP 事件输出？",
          allow_multiple: false,
          allow_free_text: false,
          options: [
            { id: "emit_events", label: "输出 MCP 事件" },
            { id: "logs_only", label: "仅记录日志" },
          ],
        },
      ],
    });

    render(<ChoiceRequestEntry entry={entry} onRespond={onRespond} />);

    expect(screen.getByText("启动自检策略？")).toBeInTheDocument();
    expect(screen.getByText("影响范围？")).toBeInTheDocument();
    expect(screen.getByText("MCP 事件输出？")).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("每次启动都自检"));
    expect(screen.getByRole("button", { name: "提交选择" })).toBeDisabled();
    fireEvent.click(screen.getByLabelText("Story/Design/Work Item 共享链路"));
    fireEvent.click(screen.getByLabelText("输出 MCP 事件"));
    fireEvent.click(screen.getByRole("button", { name: "提交选择" }));

    expect(onRespond).toHaveBeenCalledWith(entry, {
      selected_option_ids: ["self_check", "shared", "emit_events"],
      free_text: null,
      answers: [
        {
          question_id: "startup",
          selected_option_ids: ["self_check"],
          free_text: null,
        },
        {
          question_id: "scope",
          selected_option_ids: ["shared"],
          free_text: null,
        },
        {
          question_id: "mcp_events",
          selected_option_ids: ["emit_events"],
          free_text: null,
        },
      ],
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

  it("shows rejected stale choice requests without submit controls", () => {
    const onRespond = vi.fn();
    const entry = makeChoiceEntry({
      resolved: true,
      rejected: true,
      rejection_reason: "ChoiceResponse id=choice-1 not found in pending",
    });

    render(<ChoiceRequestEntry entry={entry} onRespond={onRespond} />);

    expect(screen.getByText("选择已失效")).toBeInTheDocument();
    expect(screen.getByText("ChoiceResponse id=choice-1 not found in pending")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "提交选择" })).not.toBeInTheDocument();
  });

  it("shows the choice request source", () => {
    render(<ChoiceRequestEntry entry={makeChoiceEntry({ source: "ask_user_question" })} />);

    expect(screen.getByText("AskUserQuestion")).toBeInTheDocument();
  });

  it("shows text fallback choice request source", () => {
    render(<ChoiceRequestEntry entry={makeChoiceEntry({ source: "text_fallback" })} />);

    expect(screen.getByText("文本 fallback")).toBeInTheDocument();
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

  // F-43 ①：危险命令全文可达数千字符，此前 prompt 原样铺开把选项按钮推到卡尾
  // （折叠区外），用户看不到可点控件。契约：prompt 文本 clamp 到 6 行并给出
  // 「展开全文」，选项区与提交钮紧跟 clamp 之后常显。
  it("clamps a long prompt and keeps the options right after the clamped text", () => {
    const onRespond = vi.fn();
    const entry = makeChoiceEntry({
      prompt: Array.from({ length: 30 }, (_, index) => `第 ${index + 1} 行说明`).join("\n"),
      options: [
        { id: "continue", label: "继续" },
        { id: "stop", label: "停止" },
      ],
    });

    render(<ChoiceRequestEntry entry={entry} onRespond={onRespond} />);

    const prompt = screen.getByTestId("choice-prompt-text");
    expect(prompt.className).toContain("line-clamp-6");
    const option = screen.getByLabelText("继续");
    expect(
      prompt.compareDocumentPosition(option) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    expect(screen.getByRole("button", { name: "提交选择" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "展开全文" }));

    expect(screen.getByTestId("choice-prompt-text").className).not.toContain("line-clamp-6");
    expect(screen.getByRole("button", { name: "收起全文" })).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("继续"));
    fireEvent.click(screen.getByRole("button", { name: "提交选择" }));
    expect(onRespond).toHaveBeenCalledWith(entry, {
      selected_option_ids: ["continue"],
      free_text: null,
    });
  });

  it("leaves a short prompt unclamped without an expand toggle", () => {
    render(<ChoiceRequestEntry entry={makeChoiceEntry({ prompt: "请选择下一步" })} />);

    expect(screen.getByTestId("choice-prompt-text").className).not.toContain("line-clamp-6");
    expect(screen.queryByRole("button", { name: "展开全文" })).toBeNull();
  });

  // F-43 ①：provider 的危险命令询问（"⚠️ Dangerous command:" + 命令正文 +
  // "Allow?"）此前整段当普通文本渲染，命令正文丢失等宽可读性。契约：命令正文
  // 落 <pre> 等宽块并内滚（max-h + overflow-auto + break-words），头尾文字保留。
  it("renders the dangerous command body as a scrollable monospace block", () => {
    const command = 'rm -rf "$BASE"; mkdir -p "$BASE"\nnode --test test/status-integration.test.js';
    const entry = makeChoiceEntry({
      prompt: `⚠️ Dangerous command:\n\n  ${command}\n\nAllow?`,
      options: [
        { id: "Yes", label: "Yes" },
        { id: "No", label: "No" },
      ],
    });

    render(<ChoiceRequestEntry entry={entry} onRespond={vi.fn()} />);

    const block = screen.getByTestId("choice-command-block");
    expect(block.tagName).toBe("PRE");
    expect(block.className).toContain("font-mono");
    expect(block.className).toContain("break-words");
    expect(block.className).toContain("overflow-auto");
    expect(block.className).toContain("max-h-");
    expect(block).toHaveTextContent('rm -rf "$BASE"');
    expect(block).not.toHaveTextContent("Dangerous command");
    expect(block).not.toHaveTextContent("Allow?");
    expect(screen.getByText(/Allow\?/)).toBeInTheDocument();
    // 命令块自身常显，无需展开：选项与提交钮同样直接可见。
    expect(screen.queryByRole("button", { name: "展开全文" })).toBeNull();
    expect(screen.getByLabelText("Yes")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "提交选择" })).toBeInTheDocument();
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
