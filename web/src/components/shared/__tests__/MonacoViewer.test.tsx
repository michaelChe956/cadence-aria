import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MonacoViewer } from "../MonacoViewer";

vi.mock("@monaco-editor/react", () => ({
  Editor: ({
    value,
    language,
    height,
    options,
    theme,
  }: {
    value?: string;
    language?: string;
    height?: string;
    options?: { readOnly?: boolean; wordWrap?: string; minimap?: { enabled?: boolean } };
    theme?: string;
  }) => (
    <div
      data-testid="monaco-editor"
      data-language={language}
      data-height={height}
      data-read-only={String(options?.readOnly)}
      data-word-wrap={options?.wordWrap}
      data-minimap={String(options?.minimap?.enabled)}
      data-theme={theme}
    >
      {value}
    </div>
  ),
}));

describe("MonacoViewer", () => {
  it("shows a skeleton while the editor module is loading", () => {
    render(<MonacoViewer value="# Story" height="240px" />);

    expect(screen.getByTestId("monaco-viewer-skeleton")).toHaveStyle({ height: "240px" });
  });

  it("renders a read-only markdown editor with viewer options", async () => {
    render(<MonacoViewer value="# Story\n\n正文" language="markdown" height="360px" />);

    const editor = await screen.findByTestId("monaco-editor");
    expect(editor).toHaveTextContent("# Story");
    expect(editor).toHaveAttribute("data-language", "markdown");
    expect(editor).toHaveAttribute("data-height", "360px");
    expect(editor).toHaveAttribute("data-read-only", "true");
    expect(editor).toHaveAttribute("data-word-wrap", "on");
    expect(editor).toHaveAttribute("data-minimap", "false");
    expect(editor).toHaveAttribute("data-theme", "vs");
  });

  it("uses Monaco dark theme when the system prefers dark color scheme", async () => {
    mockPrefersDarkColorScheme();

    render(<MonacoViewer value="# Story" />);

    const editor = await screen.findByTestId("monaco-editor");
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
