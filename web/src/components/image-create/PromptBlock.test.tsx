import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useImageCreateStore } from "../../state/image-create-store";
import { PromptBlock } from "./PromptBlock";

const originalState = useImageCreateStore.getState();

beforeEach(() => {
  useImageCreateStore.setState({
    ...originalState,
    params: {
      prompt: "原始 suggested prompt",
      size: "auto",
      quality: "auto",
      background: "auto",
      output_format: "png",
      input_fidelity: null,
    },
    entries: [],
  });
});

describe("PromptBlock", () => {
  it("renders the current suggested prompt and writes edits back to the store", () => {
    render(<PromptBlock />);

    const editor = screen.getByRole("textbox", { name: "建议提示词" });
    expect(editor).toHaveValue("原始 suggested prompt");
    fireEvent.change(editor, { target: { value: "修改后的提示词" } });

    expect(useImageCreateStore.getState().params.prompt).toBe("修改后的提示词");
  });

  it("shows the parse-failure system notice branch", () => {
    useImageCreateStore.setState({
      entries: [
        {
          id: "notice-1",
          type: "system_notice",
          role: "system",
          content: "本轮未产出新的建议 prompt，已保留上一版",
          timestamp: "2026-08-03T10:00:00Z",
        },
      ],
    });

    render(<PromptBlock />);

    expect(screen.getByRole("status")).toHaveTextContent(
      "本轮未产出新的建议 prompt，已保留上一版",
    );
  });
});
