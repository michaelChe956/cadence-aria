import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionRecord } from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";
import { ChatPane } from "./ChatPane";

const originalState = useImageCreateStore.getState();

beforeEach(() => {
  useImageCreateStore.setState({
    ...originalState,
    entries: [],
    isBusy: false,
    currentSession: null,
  });
});

describe("ChatPane", () => {
  it.each([
    ["image/png", "/api/image-create/sessions/session-1/images/png-image"],
    ["image/webp", "/api/image-create/sessions/session-1/images/webp-image"],
  ])("renders generation images using their endpoint URL", (mediaType, expectedSrc) => {
    useImageCreateStore.setState({
      entries: [
        {
          id: "image-1",
          type: "generation_image",
          role: "provider",
          content: "生成的图片",
          prompt: "商务插画",
          mediaType,
          imageUrl: expectedSrc,
          timestamp: "2026-08-03T10:00:00Z",
        },
      ],
    });

    render(<ChatPane />);

    expect(screen.getByRole("img", { name: "商务插画" })).toHaveAttribute(
      "src",
      expectedSrc,
    );
    expect(screen.getByRole("link", { name: "下载原图" })).toHaveAttribute(
      "href",
      expectedSrc,
    );
    expect(screen.getByRole("link", { name: "下载原图" })).toHaveAttribute(
      "download",
      `image-create-image-1.${mediaType.split("/")[1]}`,
    );
  });

  it("shows a readable placeholder when an image endpoint fails", () => {
    useImageCreateStore.setState({
      entries: [
        {
          id: "image-1",
          type: "generation_image",
          role: "provider",
          content: "生成的图片",
          prompt: "商务插画",
          mediaType: "image/png",
          imageUrl: "/api/image-create/sessions/session-1/images/missing-image",
          timestamp: "2026-08-03T10:00:00Z",
        },
      ],
    });

    render(<ChatPane />);
    fireEvent.error(screen.getByRole("img", { name: "商务插画" }));

    expect(screen.getByRole("alert")).toHaveTextContent("图片文件缺失");
  });

  it("shows readable generation errors", () => {
    useImageCreateStore.setState({
      entries: [
        {
          id: "error-1",
          type: "generation_error",
          role: "system",
          content: "图片服务暂时不可用",
          timestamp: "2026-08-03T10:00:00Z",
        },
      ],
    });

    render(<ChatPane />);
    expect(screen.getByRole("alert")).toHaveTextContent("图片服务暂时不可用");
  });

  it("uses mobile-first touch sizing and full-width message content", () => {
    useImageCreateStore.setState({
      currentSession: sessionRecord(),
      entries: [
        {
          id: "user-1",
          type: "user_message",
          role: "user",
          content: "请加强对比度",
          timestamp: "2026-08-03T10:00:00Z",
        },
      ],
    });

    render(<ChatPane />);

    expect(screen.getByRole("region", { name: "创作对话" })).toHaveClass(
      "min-h-[60vh]",
      "lg:min-h-[36rem]",
    );
    expect(screen.getByLabelText("创作消息")).toHaveClass(
      "text-base",
      "sm:text-sm",
    );
    expect(screen.getByText("请加强对比度")).toHaveClass(
      "max-w-none",
      "sm:max-w-[85%]",
    );
  });

  it("submits chat messages and disables the input while busy", () => {
    const sendMessage = vi.fn();
    useImageCreateStore.setState({
      sendMessage,
      currentSession: sessionRecord(),
    });
    const { rerender } = render(<ChatPane />);

    fireEvent.change(screen.getByLabelText("创作消息"), {
      target: { value: "请加强对比度" },
    });
    fireEvent.submit(screen.getByTestId("image-create-chat-form"));
    expect(sendMessage).toHaveBeenCalledWith("请加强对比度");

    useImageCreateStore.setState({ isBusy: true });
    rerender(<ChatPane />);
    expect(screen.getByLabelText("创作消息")).toBeDisabled();
    expect(
      screen.getByText((content) => content.includes("正在处理，请稍候")),
    ).toBeInTheDocument();
  });
});

function sessionRecord(): SessionRecord {
  return {
    session: {
      id: "session-1",
      provider_name: "claude_code",
      template: { preset: "ppt_business_illustration" },
      last_provider_session_id: null,
      current_prompt: "prompt",
      status: "active",
      created_at: "2026-08-03T10:00:00Z",
    },
    messages: [],
    prompt_blocks: [],
    generation_results: [],
    events: [],
    generation: 0,
  };
}
