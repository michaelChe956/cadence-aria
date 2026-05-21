import { fireEvent, render, screen } from "@testing-library/react";
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
