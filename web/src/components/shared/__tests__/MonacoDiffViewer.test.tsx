import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MonacoDiffViewer } from "../MonacoDiffViewer";

vi.mock("@monaco-editor/react", () => ({
  DiffEditor: ({
    original,
    modified,
    language,
    height,
    options,
    keepCurrentOriginalModel,
    keepCurrentModifiedModel,
    theme,
  }: {
    original?: string;
    modified?: string;
    language?: string;
    height?: string;
    options?: { readOnly?: boolean; renderSideBySide?: boolean; minimap?: { enabled?: boolean } };
    keepCurrentOriginalModel?: boolean;
    keepCurrentModifiedModel?: boolean;
    theme?: string;
  }) => (
    <div
      data-testid="monaco-diff-editor"
      data-language={language}
      data-height={height}
      data-read-only={String(options?.readOnly)}
      data-side-by-side={String(options?.renderSideBySide)}
      data-minimap={String(options?.minimap?.enabled)}
      data-keep-original={String(keepCurrentOriginalModel)}
      data-keep-modified={String(keepCurrentModifiedModel)}
      data-theme={theme}
    >
      <span data-testid="monaco-diff-original">{original}</span>
      <span data-testid="monaco-diff-modified">{modified}</span>
    </div>
  ),
}));

describe("MonacoDiffViewer", () => {
  it("shows a skeleton while the diff editor module is loading", () => {
    render(<MonacoDiffViewer original="# v1" modified="# v2" height="280px" />);

    expect(screen.getByTestId("monaco-diff-skeleton")).toHaveStyle({ height: "280px" });
  });

  it("renders a read-only side-by-side markdown diff", async () => {
    render(
      <MonacoDiffViewer
        original="# v1\n\n旧内容"
        modified="# v2\n\n新内容"
        language="markdown"
        height="420px"
      />,
    );

    const editor = await screen.findByTestId("monaco-diff-editor");
    expect(screen.getByTestId("monaco-diff-original")).toHaveTextContent("旧内容");
    expect(screen.getByTestId("monaco-diff-modified")).toHaveTextContent("新内容");
    expect(editor).toHaveAttribute("data-language", "markdown");
    expect(editor).toHaveAttribute("data-height", "420px");
    expect(editor).toHaveAttribute("data-read-only", "true");
    expect(editor).toHaveAttribute("data-side-by-side", "true");
    expect(editor).toHaveAttribute("data-minimap", "false");
    expect(editor).toHaveAttribute("data-keep-original", "true");
    expect(editor).toHaveAttribute("data-keep-modified", "true");
    expect(editor).toHaveAttribute("data-theme", "vs");
  });

  it("uses Monaco dark theme when the system prefers dark color scheme", async () => {
    mockPrefersDarkColorScheme();

    render(<MonacoDiffViewer original="# v1" modified="# v2" />);

    const editor = await screen.findByTestId("monaco-diff-editor");
    expect(editor).toHaveAttribute("data-theme", "vs-dark");
  });
});

function mockPrefersDarkColorScheme() {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: query === "(prefers-color-scheme: dark)",
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
}
