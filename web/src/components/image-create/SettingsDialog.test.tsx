import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MaskedSettings } from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";
import { SettingsDialog } from "./SettingsDialog";

const originalState = useImageCreateStore.getState();
const settings: MaskedSettings = {
  vendors: {
    gpt: {
      base_url: "https://api.openai.com/v1",
      api_key_masked: "sk-****1234",
      models: ["gpt-image-2.6", "gpt-image-2.6-flare"],
      default_model: "gpt-image-2.6-flare",
      recommended_models: ["gpt-image-2.6", "gpt-image-2.6-flare"],
      available: true,
    },
    minimax: {
      base_url: "https://api.minimax.chat/v1",
      api_key_masked: "mm-****5678",
      models: ["MiniMax-H3-Image"],
      default_model: "MiniMax-H3-Image",
      recommended_models: ["MiniMax-H3-Image", "MiniMax-H3-Image-2"],
      available: true,
    },
  },
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

  it("F-14 删除轮转模型所在行后轮转模型变为清单首项", async () => {
    const user = userEvent.setup();
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const models = gpt.getByLabelText("GPT 模型清单");
    await user.clear(models);
    await user.type(models, "gpt-image-2.6\n");

    expect(gpt.getByRole("combobox", { name: "GPT 轮转模型" })).toHaveTextContent(
      /^gpt-image-2\.6$/,
    );
  });

  it("提交改动过的模型清单与轮转模型", async () => {
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
            default_model: "gpt-image-2.6",
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
