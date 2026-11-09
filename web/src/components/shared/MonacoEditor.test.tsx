import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MonacoEditor } from "./MonacoEditor";

vi.mock("@monaco-editor/react", () => ({
  // ★`monaco-local-setup.ts` 会在模块加载期调用 `loader.config`（本地 monaco 装配）
  //   ⇒ mock 必须同时提供 `loader`，否则收集阶段抛
  //   「No "loader" export is defined on the "@monaco-editor/react" mock」。
  loader: {
    config: vi.fn(),
  },
  Editor: ({
    value,
    language,
    height,
    options,
    theme,
    onChange,
  }: {
    value?: string;
    language?: string;
    height?: string;
    options?: { readOnly?: boolean; domReadOnly?: boolean; wordWrap?: string };
    theme?: string;
    onChange?: (value: string | undefined) => void;
  }) => (
    <div
      data-testid="monaco-editor"
      data-language={language}
      data-height={height}
      data-read-only={String(options?.readOnly)}
      data-dom-read-only={String(options?.domReadOnly)}
      data-theme={theme}
    >
      <button type="button" onClick={() => onChange?.("编辑后的内容")}>
        simulate-typing
      </button>
      {value}
    </div>
  ),
}));

describe("MonacoEditor", () => {
  it("编辑输入触发 onChange 回调", async () => {
    const onChange = vi.fn();
    render(<MonacoEditor value="# 初始内容" onChange={onChange} />);

    const editor = await screen.findByTestId("monaco-editor");
    fireEvent.click(screen.getByRole("button", { name: "simulate-typing" }));

    expect(editor).toBeInTheDocument();
    expect(onChange).toHaveBeenCalledWith("编辑后的内容");
  });

  it("未传 onChange 时渲染为只读且不报错", async () => {
    render(<MonacoEditor value="# 只读内容" />);

    const editor = await screen.findByTestId("monaco-editor");
    expect(editor).toHaveAttribute("data-read-only", "true");
    // 可写编辑器不得沿用 MonacoViewer 的 domReadOnly（会拦掉 DOM 输入事件）。
    expect(editor).toHaveAttribute("data-dom-read-only", "undefined");
    fireEvent.click(screen.getByRole("button", { name: "simulate-typing" }));
  });

  it("传 onChange 时可写并沿用 markdown 默认语言与 400px 默认高度", async () => {
    render(<MonacoEditor value="# 内容" onChange={vi.fn()} />);

    const editor = await screen.findByTestId("monaco-editor");
    expect(editor).toHaveAttribute("data-read-only", "false");
    expect(editor).toHaveAttribute("data-language", "markdown");
    expect(editor).toHaveAttribute("data-height", "400px");
    expect(editor).toHaveAttribute("data-theme", "vs");
  });
});
