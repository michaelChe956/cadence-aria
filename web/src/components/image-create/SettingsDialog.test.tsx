import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MaskedSettings } from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";
import { SettingsDialog } from "./SettingsDialog";

const originalState = useImageCreateStore.getState();
// 单家视图用中间变量构造（非常量字面量）：字段增删时不会触发多余属性检查。
const gptVendor = {
  base_url: "https://api.openai.com/v1",
  api_key_masked: "sk-****1234",
  models: ["gpt-image-2.6", "gpt-image-2.6-flare"],
  recommended_models: ["gpt-image-2.6", "gpt-image-2.6-flare"],
  available: true,
};
const minimaxVendor = {
  base_url: "https://api.minimax.chat/v1",
  api_key_masked: "mm-****5678",
  models: ["MiniMax-H3-Image"],
  recommended_models: ["MiniMax-H3-Image", "MiniMax-H3-Image-2"],
  available: true,
};
const settings: MaskedSettings = {
  vendors: { gpt: gptVendor, minimax: minimaxVendor },
  defaults: {
    size: "1024x1024",
    quality: "medium",
    background: "opaque",
    output_format: "png",
  },
};

function renderDialog() {
  const loadSettings = vi.fn(async () => {
    useImageCreateStore.setState({ settings });
  });
  const saveSettings = vi.fn().mockResolvedValue(undefined);
  useImageCreateStore.setState({ loadSettings, saveSettings });

  render(<SettingsDialog onClose={vi.fn()} />);

  return { loadSettings, saveSettings };
}

beforeEach(() => {
  useImageCreateStore.setState({
    ...originalState,
    settings: null,
    error: null,
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

describe("SettingsDialog", () => {
  it("loads settings and displays each vendor's masked API key", async () => {
    const { loadSettings } = renderDialog();

    const gpt = within(await screen.findByRole("region", { name: "GPT" }));
    const minimax = within(screen.getByRole("region", { name: "MiniMax" }));

    expect(gpt.getByLabelText("GPT API Key")).toHaveValue("sk-****1234");
    expect(minimax.getByLabelText("MiniMax API Key")).toHaveValue("mm-****5678");
    expect(gpt.getByLabelText("GPT base_url")).toHaveValue(
      "https://api.openai.com/v1",
    );
    expect(loadSettings).toHaveBeenCalledTimes(1);
  });

  it("sends no vendor and no defaults when nothing changed", async () => {
    const user = userEvent.setup();
    const { loadSettings, saveSettings } = renderDialog();
    await screen.findByRole("region", { name: "GPT" });

    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({});
    });
    expect(loadSettings).toHaveBeenCalledTimes(2);
    expect(screen.getByText("设置已保存")).toBeInTheDocument();
  });

  it("F-11 只改 MiniMax 的 key 时请求只含 vendors.minimax", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const minimax = within(await screen.findByRole("region", { name: "MiniMax" }));

    const apiKey = minimax.getByLabelText("MiniMax API Key");
    await user.clear(apiKey);
    await user.type(apiKey, "mm-new-secret");
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({
        vendors: {
          minimax: { api_key_action: "replace", api_key: "mm-new-secret" },
        },
      });
    });
    const request = saveSettings.mock.calls[0]?.[0];
    expect(request?.vendors?.gpt).toBeUndefined();
  });

  it("F-12 点 GPT 的清除按钮时只提交 GPT 的 clear 动作", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    await user.click(gpt.getByRole("button", { name: "清除 key" }));
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({
        vendors: { gpt: { api_key_action: "clear" } },
      });
    });
    const request = saveSettings.mock.calls[0]?.[0];
    expect(request?.vendors?.minimax).toBeUndefined();
    expect(request?.vendors?.gpt).not.toHaveProperty("api_key");
  });

  it("F-13 恢复推荐清单后文本框内容等于 recommended_models", async () => {
    const user = userEvent.setup();
    renderDialog();
    const minimax = within(await screen.findByRole("region", { name: "MiniMax" }));

    const models = minimax.getByLabelText("MiniMax 模型清单");
    await user.clear(models);
    await user.click(minimax.getByRole("button", { name: "恢复推荐清单" }));

    expect(models).toHaveValue(
      settings.vendors.minimax.recommended_models.join("\n"),
    );
  });

  it("提交改动过的模型清单", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const models = gpt.getByLabelText("GPT 模型清单");
    await user.clear(models);
    await user.type(models, "gpt-image-2.6\n");
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({
        vendors: {
          gpt: {
            api_key_action: "retain",
            models: ["gpt-image-2.6"],
          },
        },
      });
    });
  });

  it("共享的默认参数分区照旧读写", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    await screen.findByRole("region", { name: "GPT" });

    await selectCustomOption(user, "默认质量", "high");
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({
        defaults: {
          ...settings.defaults,
          quality: "high",
        },
      });
    });
  });

  it("阻断无效的 base_url 且不调用 store", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    await user.clear(gpt.getByLabelText("GPT base_url"));
    await user.type(gpt.getByLabelText("GPT base_url"), "http://example.com/v1");
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    expect(
      await screen.findByText("base_url 必须使用 HTTPS，或使用 localhost/loopback IP"),
    ).toBeInTheDocument();
    expect(saveSettings).not.toHaveBeenCalled();
  });

  it("后端校验失败的中文信息原样显示", async () => {
    const user = userEvent.setup();
    const loadSettings = vi.fn(async () => {
      useImageCreateStore.setState({ settings });
    });
    const saveSettings = vi.fn().mockRejectedValue(new Error("模型清单包含重复 id"));
    useImageCreateStore.setState({ loadSettings, saveSettings });

    render(<SettingsDialog onClose={vi.fn()} />);
    await screen.findByRole("region", { name: "GPT" });

    await user.click(screen.getByRole("button", { name: "保存设置" }));

    expect(await screen.findByText("模型清单包含重复 id")).toBeInTheDocument();
  });
});

/** 用自定义设置渲染弹窗（供视觉层用例构造不同配置状态）。 */
function renderDialogWithSettings(custom: MaskedSettings) {
  const loadSettings = vi.fn(async () => {
    useImageCreateStore.setState({ settings: custom });
  });
  const saveSettings = vi.fn().mockResolvedValue(undefined);
  useImageCreateStore.setState({ loadSettings, saveSettings });

  render(<SettingsDialog onClose={vi.fn()} />);

  return { loadSettings, saveSettings };
}

describe("SettingsDialog 视觉与信息层级", () => {
  it("分区字段标签不再重复供应方前缀", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    expect(gpt.getByText("Base URL")).toBeInTheDocument();
    expect(gpt.getByText("API Key")).toBeInTheDocument();
    expect(gpt.getByText("模型清单")).toBeInTheDocument();
    expect(gpt.queryByText(/GPT base_url/)).toBeNull();
    expect(gpt.queryByText(/GPT API Key/)).toBeNull();
    expect(gpt.queryByText(/GPT 模型清单/)).toBeNull();
  });

  it("分区只剩三个字段：Base URL / API Key / 模型清单", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    expect(gpt.getByLabelText("GPT base_url")).toBeInTheDocument();
    expect(gpt.getByLabelText("GPT API Key")).toBeInTheDocument();
    expect(gpt.getByLabelText("GPT 模型清单")).toBeInTheDocument();
    expect(gpt.queryByText("轮转模型")).toBeNull();
    expect(gpt.queryByRole("combobox")).toBeNull();
  });

  it("分区标题带图标圆角底与已配置状态 chip", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    expect(gpt.getByText("已配置")).toBeInTheDocument();
    expect(gpt.getByText("GPT").tagName).toBe("H3");
  });

  it("缺少凭据的供应方显示未配置 chip", async () => {
    renderDialogWithSettings({
      ...settings,
      vendors: {
        ...settings.vendors,
        minimax: { ...settings.vendors.minimax, base_url: "", api_key_masked: "" },
      },
    });
    const minimax = within(await screen.findByRole("region", { name: "MiniMax" }));

    expect(minimax.getByText("未配置")).toBeInTheDocument();
    expect(
      minimax.getByText("尚未配置 key，填写后保存生效"),
    ).toBeInTheDocument();
  });

  it("状态 chip 随编辑态变化：清除 key 后变为未配置", async () => {
    const user = userEvent.setup();
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    expect(gpt.getByText("已配置")).toBeInTheDocument();
    await user.click(gpt.getByRole("button", { name: "清除 key" }));
    expect(gpt.getByText("未配置")).toBeInTheDocument();
  });

  it("输入控件与下拉使用同一套边框、圆角与内边距", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const controls = [
      gpt.getByLabelText("GPT base_url"),
      gpt.getByLabelText("GPT API Key"),
      gpt.getByLabelText("GPT 模型清单"),
      screen.getByRole("combobox", { name: "默认质量" }),
    ];
    for (const control of controls) {
      expect(control.className).toContain("rounded-xl");
      expect(control.className).toContain("border-[var(--aria-line)]");
      expect(control.className).toContain("px-3.5");
      expect(control.className).toContain("focus-visible:ring-2");
      expect(control.className).toContain("duration-200");
    }
  });

  it("模型清单 textarea 使用等宽字体、最小高度且可纵向缩放", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const models = gpt.getByLabelText("GPT 模型清单");
    expect(models.className).toContain("font-mono");
    expect(models.className).toContain("min-h-[10rem]");
    expect(models.className).toContain("resize-y");
  });

  it("清除 key 按钮与 API Key 输入框同一行且状态说明在下方", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const apiKey = gpt.getByLabelText("GPT API Key");
    const clearButton = gpt.getByRole("button", { name: "清除 key" });
    expect(clearButton.parentElement).toContainElement(apiKey);
    expect(
      gpt.getByText("已保存 sk-****1234，留空保持不变"),
    ).toBeInTheDocument();
  });

  it("恢复推荐清单按钮与模型清单标签同一行", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const row = gpt.getByRole("button", { name: "恢复推荐清单" }).parentElement;
    expect(row).toContainElement(gpt.getByText("模型清单"));
  });

  it("分区卡片使用 2px 中灰边框，默认参数分区用细分隔线", async () => {
    renderDialog();
    const gpt = await screen.findByRole("region", { name: "GPT" });

    expect(gpt.className).toContain("border-2");
    expect(gpt.className).toContain("border-[var(--aria-border-muted)]");
    expect(gpt.className).toContain("rounded-2xl");

    const legend = screen.getByText("默认生成参数");
    const divider = legend.closest("fieldset")?.parentElement;
    expect(divider?.className).toContain("border-t");
    expect(divider?.className).toContain("border-[var(--aria-line)]");
  });

  it("默认参数两列网格仅在 sm 断点以上生效（375px 单列）", async () => {
    renderDialog();
    await screen.findByRole("region", { name: "GPT" });

    const legend = screen.getByText("默认生成参数");
    const grid = legend.closest("fieldset")?.querySelector("div.grid");
    expect(grid?.className).toContain("sm:grid-cols-2");
    expect(grid?.className.split(" ")).not.toContain("grid-cols-2");
  });

  it("轮转模型字段已移除：分区不再出现轮转下拉与轮转说明", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    expect(gpt.queryByRole("combobox")).toBeNull();
    expect(gpt.queryByText(/自动轮转/)).toBeNull();
  });
});
