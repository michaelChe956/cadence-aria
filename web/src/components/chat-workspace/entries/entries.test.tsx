import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryRenderer } from "../ChatEntryRenderer";
import { ErrorEntry } from "./ErrorEntry";
import { ExecutionEventEntry } from "./ExecutionEventEntry";
import { PermissionRequestEntry } from "./PermissionRequestEntry";
import { PermissionResponseEntry } from "./PermissionResponseEntry";
import { ProviderStreamEntry } from "./ProviderStreamEntry";
import { UserContextEntry } from "./UserContextEntry";

describe("chat workspace entries", () => {
  it("renders user context entries", () => {
    const entry = makeEntry({
      type: "context_note",
      role: "user",
      content: "需要支持手机号登录",
    });

    render(<UserContextEntry entry={entry} />);

    expect(screen.getByText("你")).toBeInTheDocument();
    expect(screen.getByText("需要支持手机号登录")).toBeInTheDocument();
  });

  it("renders provider stream entries as markdown-like blocks", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content: "# Story Spec\n\n支持手机号登录。",
    });

    render(<ProviderStreamEntry entry={entry} />);

    expect(screen.getByRole("heading", { name: "Story Spec" })).toBeInTheDocument();
    expect(screen.getByText("支持手机号登录。")).toBeInTheDocument();
  });

  it("renders provider stream content with escaped and single newlines as readable blocks", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content: "# Story Spec\\n## 范围\\n实现爬楼梯问题\\n- n=1\\n- n=2",
    });

    render(<ProviderStreamEntry entry={entry} />);

    expect(screen.getByRole("heading", { name: "Story Spec" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "范围" })).toBeInTheDocument();
    expect(screen.getByText(/实现爬楼梯问题/)).toBeInTheDocument();
    expect(screen.getByText(/n=1/)).toBeInTheDocument();
  });

  it("renders provider stream markdown semantics inside message bubbles", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content:
        "## 范围\n\n- 支持 **粗体** 和 `inline code`\n- 查看 [设计说明](https://example.com/design)",
    });

    render(<ProviderStreamEntry entry={entry} />);

    expect(screen.getByRole("heading", { name: "范围" })).toBeInTheDocument();
    const list = screen.getByRole("list");
    expect(list).toBeInTheDocument();
    expect(within(list).getAllByRole("listitem")[0]).toHaveTextContent(
      "支持 粗体 和 inline code",
    );
    expect(screen.getByText("粗体").tagName).toBe("STRONG");
    expect(screen.getByText("inline code").tagName).toBe("CODE");
    expect(screen.getByRole("link", { name: "设计说明" })).toHaveAttribute(
      "href",
      "https://example.com/design",
    );
  });

  it("does not inject raw HTML or unsafe markdown links in message bubbles", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content:
        '正文\n\n<script>alert("x")</script>\n\n<img src=x onerror=alert(1)>\n\n[危险链接](javascript:alert(1))',
    });

    const { container } = render(<ProviderStreamEntry entry={entry} />);

    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByText('<script>alert("x")</script>')).toBeInTheDocument();
    expect(screen.getByText("<img src=x onerror=alert(1)>")).toBeInTheDocument();
    expect(screen.getByText("危险链接")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "危险链接" })).not.toBeInTheDocument();
  });

  it("breaks long provider prose into sentence lines when the provider streams one dense line", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content:
        "仓库规则已确认。 我现在进入红灯阶段。 红灯已成立：python -m unittest discover -s tests 因 ModuleNotFoundError 失败。 绿色阶段测试已通过。",
    });

    render(<ProviderStreamEntry entry={entry} />);

    const prose = screen.getByText(/仓库规则已确认/);
    expect(prose.textContent).toBe(
      "仓库规则已确认。\n我现在进入红灯阶段。\n红灯已成立：python -m unittest discover -s tests 因 ModuleNotFoundError 失败。\n绿色阶段测试已通过。",
    );
  });

  it("renders execution event entries with command detail", () => {
    const entry = makeEntry({
      type: "execution_event",
      role: "system",
      content: "读取认证模块 · exit code 0",
      metadata: { command: "sed -n '1,120p' src/auth.rs", output: "ok" },
    });

    render(<ExecutionEventEntry entry={entry} />);

    expect(screen.getByText("执行事件")).toBeInTheDocument();
    expect(screen.getByText("读取认证模块 · exit code 0")).toBeInTheDocument();
    expect(screen.getByText("sed -n '1,120p' src/auth.rs")).toBeInTheDocument();
    expect(screen.getByText("ok")).toBeInTheDocument();
  });

  it("renders permission request entries and emits response actions", () => {
    const onRespond = vi.fn();
    const entry = makeEntry({
      type: "permission_request",
      role: "system",
      content: "shell · cargo test",
      metadata: {
        request_id: "permission-1",
        request: {
          tool_name: "shell",
          description: "cargo test",
          risk_level: "medium",
        },
        risk_level: "medium",
      },
    });

    render(<PermissionRequestEntry entry={entry} onRespond={onRespond} />);

    fireEvent.click(screen.getByRole("button", { name: "允许" }));

    expect(screen.getByText("shell")).toBeInTheDocument();
    expect(screen.getByText("cargo test")).toBeInTheDocument();
    expect(onRespond).toHaveBeenCalledWith(entry, true);
  });

  it("renders permission response entries as compact tags", () => {
    const entry = makeEntry({
      type: "permission_response",
      role: "user",
      content: "已允许 shell",
      metadata: { approved: true },
    });

    render(<PermissionResponseEntry entry={entry} />);

    expect(screen.getByText("已允许 shell")).toBeInTheDocument();
  });

  it("renders error entries with code metadata", () => {
    const entry = makeEntry({
      type: "error",
      role: "system",
      content: "阶段不允许",
      metadata: { code: "INVALID_MESSAGE_FOR_STAGE" },
    });

    render(<ErrorEntry entry={entry} />);

    expect(screen.getByText("错误 INVALID_MESSAGE_FOR_STAGE")).toBeInTheDocument();
    expect(screen.getByText("阶段不允许")).toBeInTheDocument();
  });

  it("renders analyst verdict entries with fix hints and human questions", () => {
    render(
      <ChatEntryRenderer
        entry={makeEntry({
          type: "analyst_verdict",
          role: "analyst",
          content: "测试仍失败",
          metadata: {
            verdict: "needs_fix",
            fix_hints: ["补充 climb_stairs 动态规划实现", "覆盖 n=10"],
          },
        })}
      />,
    );

    expect(screen.getByTestId("analyst-verdict-entry")).toHaveTextContent("需要修复");
    expect(screen.getByTestId("analyst-verdict-entry")).toHaveTextContent("测试仍失败");
    expect(screen.getByText("补充 climb_stairs 动态规划实现")).toBeInTheDocument();

    render(
      <ChatEntryRenderer
        entry={makeEntry({
          id: "entry-2",
          type: "analyst_verdict",
          role: "analyst",
          content: "需要人工补充",
          metadata: {
            verdict: "needs_human_input",
            questions: ["n 的输入范围是多少？"],
          },
        })}
      />,
    );

    expect(screen.getByText("需要人工输入")).toBeInTheDocument();
    expect(screen.getByText("n 的输入范围是多少？")).toBeInTheDocument();
  });

  it("dispatches entries through the renderer", () => {
    const onRespond = vi.fn();
    const entry = makeEntry({
      type: "permission_request",
      role: "system",
      content: "shell · cargo test",
      metadata: {
        request: {
          tool_name: "shell",
          description: "cargo test",
          risk_level: "medium",
        },
        risk_level: "medium",
      },
    });

    render(<ChatEntryRenderer entry={entry} onPermissionResponse={onRespond} />);

    fireEvent.click(screen.getByRole("button", { name: "拒绝" }));

    expect(onRespond).toHaveBeenCalledWith(entry, false);
  });
});

function makeEntry(overrides: Partial<ChatEntry>): ChatEntry {
  return {
    id: "entry-1",
    type: "context_note",
    role: "user",
    content: "",
    timestamp: "2026-05-21T10:00:00Z",
    ...overrides,
  } as ChatEntry;
}
