import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  MaskedSettings,
  MaskedVendorSettings,
  SessionRecord,
} from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";
import { ParamsPanel } from "./ParamsPanel";

const originalState = useImageCreateStore.getState();

function vendorSettings(
  overrides: Partial<MaskedVendorSettings> = {},
): MaskedVendorSettings {
  return {
    base_url: "https://api.openai.com/v1",
    api_key_masked: "sk-****1234",
    models: ["gpt-image-2.6", "gpt-image-2.6-flare"],
    default_model: "gpt-image-2.6-flare",
    recommended_models: ["gpt-image-2.6", "gpt-image-2.6-flare"],
    available: true,
    ...overrides,
  };
}

function maskedSettings(
  overrides: Partial<MaskedSettings["vendors"]> = {},
): MaskedSettings {
  return {
    vendors: {
      gpt: vendorSettings(),
      minimax: vendorSettings({
        base_url: "https://api.minimax.chat/v1",
        api_key_masked: "mm-****5678",
        models: ["MiniMax-H3-Image"],
        default_model: "MiniMax-H3-Image",
        recommended_models: ["MiniMax-H3-Image"],
      }),
      ...overrides,
    },
    defaults: {
      size: "auto",
      quality: "auto",
      background: "auto",
      output_format: "png",
    },
  };
}

/** 凭据为空的供应方：available 仍为 true，使断言只能由凭据判据满足。 */
const unconfiguredVendor = vendorSettings({
  base_url: "",
  api_key_masked: "",
  models: [],
  default_model: "",
  available: true,
});

beforeEach(() => {
  useImageCreateStore.setState({
    ...originalState,
    params: {
      prompt: "画一张商务插图",
      size: "auto",
      quality: "auto",
      background: "auto",
      output_format: "png",
      model: "auto",
      use_previous: true,
    },
    settings: maskedSettings(),
    entries: [],
    referenceImage: null,
    isBusy: false,
  });
});

async function selectCustomOption(
  user: UserEvent,
  label: string,
  option: string,
) {
  await user.click(screen.getByRole("combobox", { name: label }));
  await user.click(screen.getByRole("option", { name: option }));
}

describe("ParamsPanel", () => {
  it("keeps generation visible while collapsing mobile parameters by default", async () => {
    const user = userEvent.setup();
    useImageCreateStore.setState({ currentSession: sessionRecord() });

    render(<ParamsPanel />);

    const toggle = screen.getByRole("button", { name: "展开生成参数" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByRole("combobox", { name: "尺寸" }).parentElement?.parentElement)
      .toHaveClass("hidden", "lg:grid");
    expect(screen.getByRole("button", { name: "生成图片" })).toHaveClass(
      "min-h-11",
    );

    await user.click(toggle);

    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("button", { name: "收起生成参数" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "尺寸" }).parentElement?.parentElement)
      .toHaveClass("grid");
  });

  it("binds all parameter selects to setParams and only generates on button click", async () => {
    const user = userEvent.setup();
    const setParams = vi.fn();
    const generate = vi.fn().mockResolvedValue(undefined);
    useImageCreateStore.setState({
      setParams,
      generate,
      currentSession: sessionRecord(),
    });

    render(<ParamsPanel />);

    await selectCustomOption(user, "尺寸", "1536x1024");
    await selectCustomOption(user, "质量", "high");
    await selectCustomOption(user, "背景", "transparent");
    await selectCustomOption(user, "输出格式", "webp");

    expect(setParams).toHaveBeenCalledWith({ size: "1536x1024" });
    expect(setParams).toHaveBeenCalledWith({ quality: "high" });
    expect(setParams).toHaveBeenCalledWith({ background: "transparent" });
    expect(setParams).toHaveBeenCalledWith({ output_format: "webp" });
    expect(generate).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "生成图片" }));
    expect(generate).toHaveBeenCalledTimes(1);
  });

  it("handles a rejected generation promise at the click boundary", async () => {
    const rejected = Promise.reject(new Error("生成失败"));
    void rejected.catch(() => {});
    const catchSpy = vi.spyOn(rejected, "catch");
    useImageCreateStore.setState({
      generate: vi.fn(() => rejected),
      currentSession: sessionRecord(),
    });

    render(<ParamsPanel />);
    fireEvent.click(screen.getByRole("button", { name: "生成图片" }));

    await waitFor(() => expect(catchSpy).toHaveBeenCalledTimes(1));
  });

  it("F-07 列出自动轮转与已配置凭据的供应方模型，未配置的供应方给出提示", async () => {
    const user = userEvent.setup();
    useImageCreateStore.setState({
      settings: maskedSettings({ minimax: unconfiguredVendor }),
    });

    render(<ParamsPanel />);

    await user.click(screen.getByRole("combobox", { name: "模型" }));

    expect(
      screen.getByRole("option", { name: "自动轮转（GPT ⇄ MiniMax）" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: "GPT · gpt-image-2.6" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("option", { name: "MiniMax · MiniMax-H3-Image" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("MiniMax 未配置，可在设置中添加")).toBeInTheDocument();
  });

  it("F-08 显式选择 MiniMax 模型后质量与背景置灰", async () => {
    const user = userEvent.setup();
    useImageCreateStore.setState({ settings: maskedSettings() });

    render(<ParamsPanel />);

    expect(screen.getByRole("combobox", { name: "质量" })).not.toBeDisabled();
    expect(screen.getByText("仅对 GPT 生效")).toBeInTheDocument();

    await selectCustomOption(user, "模型", "MiniMax · MiniMax-H3-Image");

    expect(screen.getByRole("combobox", { name: "质量" })).toBeDisabled();
    expect(screen.getByRole("combobox", { name: "背景" })).toBeDisabled();
    expect(screen.getByText("MiniMax 不支持该参数")).toBeInTheDocument();
    expect(screen.queryByText("仅对 GPT 生效")).not.toBeInTheDocument();
  });

  it("F-09 无生成结果或已选上传参考图时携带上一张图开关置灰", () => {
    useImageCreateStore.setState({ settings: maskedSettings(), entries: [] });

    const { rerender } = render(<ParamsPanel />);

    const toggle = screen.getByRole("checkbox", {
      name: "基于上一张图继续修改",
    });
    expect(toggle).toBeDisabled();
    expect(toggle).toBeChecked();
    expect(screen.getByText("首张图片将直接生成")).toBeInTheDocument();

    useImageCreateStore.getState().setReferenceImage(
      new File(["image"], "reference.png", { type: "image/png" }),
    );
    rerender(<ParamsPanel />);

    expect(
      screen.getByRole("checkbox", { name: "基于上一张图继续修改" }),
    ).toBeDisabled();
    expect(screen.getByText("已使用上传的参考图")).toBeInTheDocument();

    useImageCreateStore.setState({
      referenceImage: null,
      entries: [generationImageEntry()],
    });
    rerender(<ParamsPanel />);

    expect(
      screen.getByRole("checkbox", { name: "基于上一张图继续修改" }),
    ).not.toBeDisabled();
    expect(screen.queryByText("首张图片将直接生成")).not.toBeInTheDocument();
  });

  it("F-09b 点击携带上一张图开关后翻转 params.use_previous", async () => {
    const user = userEvent.setup();
    useImageCreateStore.setState({
      settings: maskedSettings(),
      entries: [generationImageEntry()],
      referenceImage: null,
    });

    render(<ParamsPanel />);

    await user.click(screen.getByRole("checkbox", { name: "基于上一张图继续修改" }));

    expect(useImageCreateStore.getState().params.use_previous).toBe(false);
  });

  it("F-10 等待提示不含具体模型名且页面没有保真度控件", () => {
    useImageCreateStore.setState({ settings: maskedSettings(), isBusy: true });

    render(<ParamsPanel />);

    expect(screen.getByText(/图片生成通常需要 1-3 分钟/)).toBeInTheDocument();
    expect(screen.queryByText(/gpt-image-2/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("combobox", { name: "参考图保真度" }),
    ).not.toBeInTheDocument();
  });
});

function generationImageEntry() {
  return {
    id: "image-1",
    type: "generation_image" as const,
    role: "provider" as const,
    content: "生成的图片",
    prompt: "画一张商务插图",
    mediaType: "image/png",
    imageUrl: "/api/image-create/sessions/session-1/images/image-1",
    timestamp: "2026-08-03T10:00:00Z",
  };
}

function sessionRecord(): SessionRecord {
  return {
    session: {
      id: "session-1",
      provider_name: "claude_code",
      template: { preset: "ppt_business_illustration" },
      last_provider_session_id: null,
      current_prompt: "画一张商务插图",
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
