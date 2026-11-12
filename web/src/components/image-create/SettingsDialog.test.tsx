import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ImageVendor, MaskedSettings } from "../../api/types/image-create";
import {
  useImageCreateStore,
  type ImageVendorConnectivityState,
} from "../../state/image-create-store";
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

function renderDialog(
  connectivity: Partial<Record<ImageVendor, ImageVendorConnectivityState>> = {},
) {
  const loadSettings = vi.fn(async () => {
    useImageCreateStore.setState({ settings });
  });
  const saveSettings = vi.fn().mockResolvedValue(undefined);
  const testVendorConnectivity = vi.fn().mockResolvedValue(undefined);
  useImageCreateStore.setState({
    loadSettings,
    saveSettings,
    testVendorConnectivity,
    connectivity: {
      gpt: { phase: "idle", result: null, error: null },
      minimax: { phase: "idle", result: null, error: null },
      ...connectivity,
    },
    modelPools: {
      gpt: { gateway: [], candidates: [] },
      minimax: { gateway: [], candidates: [] },
    },
  });

  render(<SettingsDialog onClose={vi.fn()} />);

  return { loadSettings, saveSettings, testVendorConnectivity };
}

beforeEach(() => {
  useImageCreateStore.setState({
    ...originalState,
    settings: null,
    error: null,
    modelPools: {
      gpt: { gateway: [], candidates: [] },
      minimax: { gateway: [], candidates: [] },
    },
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
  it("loads settings and shows each vendor's masked API key as a hint", async () => {
    const { loadSettings } = renderDialog();

    const gpt = within(await screen.findByRole("region", { name: "GPT" }));
    const minimax = within(screen.getByRole("region", { name: "MiniMax" }));

    // 脱敏串只作提示，绝不进输入框：用户改一个字就会把它当真 key 存回去。
    expect(gpt.getByLabelText("GPT API Key")).toHaveValue("");
    expect(gpt.getByLabelText("GPT API Key")).toHaveAttribute(
      "placeholder",
      "已保存 sk-****1234",
    );
    expect(minimax.getByLabelText("MiniMax API Key")).toHaveValue("");
    expect(minimax.getByLabelText("MiniMax API Key")).toHaveAttribute(
      "placeholder",
      "已保存 mm-****5678",
    );
    expect(gpt.getByLabelText("GPT base_url")).toHaveValue(
      "https://api.openai.com/v1",
    );
    expect(loadSettings).toHaveBeenCalledTimes(1);
  });

  it("在 key 框里改一个字不会把脱敏串当作新 key 提交", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const apiKey = gpt.getByLabelText("GPT API Key");
    await user.type(apiKey, "X");
    expect(apiKey).toHaveValue("X");
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({
        vendors: { gpt: { api_key_action: "replace", api_key: "X" } },
      });
    });
  });

  it("不碰 key 框直接保存时该家为 retain 且请求里不含 api_key", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    // 只动模型清单，key 框保持空白（表示沿用已保存的那把）。
    await user.click(gpt.getByRole("button", { name: "已选 2 项" }));
    await user.click(gpt.getByRole("checkbox", { name: "gpt-image-2.6" }));
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({
        vendors: {
          gpt: {
            api_key_action: "retain",
            models: ["gpt-image-2.6-flare"],
          },
        },
      });
    });
    const request = saveSettings.mock.calls[0]?.[0];
    expect(request?.vendors?.gpt).not.toHaveProperty("api_key");
  });

  it("在 key 框里输入后又删空时回到 retain 且请求里不含 api_key", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const apiKey = gpt.getByLabelText("GPT API Key");
    await user.type(apiKey, "sk-typed");
    expect(apiKey).toHaveValue("sk-typed");
    // 删空 ⇒ 回到「沿用已保存的 key」，输入框里不留任何明文。
    await user.clear(apiKey);
    expect(apiKey).toHaveValue("");

    // 顺带改一处模型清单，让该家进入保存请求（其余都没动时整家不入请求）。
    await user.click(gpt.getByRole("button", { name: "已选 2 项" }));
    await user.click(gpt.getByRole("checkbox", { name: "gpt-image-2.6" }));
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledTimes(1);
    });
    const request = saveSettings.mock.calls[0]?.[0];
    expect(request?.vendors?.gpt).toEqual({
      api_key_action: "retain",
      models: ["gpt-image-2.6-flare"],
    });
    // 输入过又删空 ⇒ 既不替换也不清空：请求里不能出现 api_key 字段。
    expect(request?.vendors?.gpt).not.toHaveProperty("api_key");
    expect(apiKey).toHaveValue("");
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

  it("F-13 恢复推荐清单把推荐模型并入候选池并勾选", async () => {
    const user = userEvent.setup();
    renderDialog();
    const minimax = within(await screen.findByRole("region", { name: "MiniMax" }));

    await user.click(minimax.getByRole("button", { name: "恢复推荐清单" }));
    await user.click(minimax.getByRole("button", { name: "已选 2 项" }));

    expect(
      minimax.getByRole("checkbox", { name: "MiniMax-H3-Image-2" }),
    ).toBeChecked();
  });

  it("取消勾选后保存载荷少一项", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    await user.click(gpt.getByRole("button", { name: "已选 2 项" }));
    await user.click(gpt.getByRole("checkbox", { name: "gpt-image-2.6" }));
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({
        vendors: {
          gpt: {
            api_key_action: "retain",
            models: ["gpt-image-2.6-flare"],
          },
        },
      });
    });
  });

  it("添加自定义模型后进入候选池并勾选", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    await user.click(gpt.getByRole("button", { name: "已选 2 项" }));
    await user.type(gpt.getByLabelText("添加自定义模型"), "custom-model");
    await user.click(gpt.getByRole("button", { name: "添加" }));

    expect(
      gpt.getByRole("checkbox", { name: "custom-model" }),
    ).toBeChecked();
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({
        vendors: {
          gpt: {
            api_key_action: "retain",
            models: ["gpt-image-2.6", "gpt-image-2.6-flare", "custom-model"],
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
      gpt.getByRole("button", { name: "已选 2 项" }),
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

  it("模型清单不再是 textarea：改为复选下拉框", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    expect(gpt.queryByRole("textbox", { name: "GPT 模型清单" })).toBeNull();
    expect(gpt.getByRole("group", { name: "GPT 模型清单" })).toBeInTheDocument();
    expect(gpt.getByRole("button", { name: "已选 2 项" })).toBeInTheDocument();
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

describe("SettingsDialog Base URL 示例与连通性探测", () => {
  it("给出各家的 Base URL 示例与「只填主机根路径」的说明", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));
    const minimax = within(screen.getByRole("region", { name: "MiniMax" }));

    expect(gpt.getByLabelText("GPT base_url")).toHaveAttribute(
      "placeholder",
      "https://api.openai.com",
    );
    expect(minimax.getByLabelText("MiniMax base_url")).toHaveAttribute(
      "placeholder",
      "https://api.minimax.io",
    );
    const hint =
      "只填到主机根路径，不要带 /v1 或 /v1/images；系统会自动拼成 /v1/images/generations。";
    expect(gpt.getByText(hint)).toBeInTheDocument();
    expect(minimax.getByText(hint)).toBeInTheDocument();
  });

  it("测试连通按钮位于 Base URL 附近而不是标题行", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const button = gpt.getByRole("button", { name: "测试连通" });
    const baseUrl = gpt.getByLabelText("GPT base_url");
    expect(baseUrl.parentElement).toContainElement(button);
  });

  it("用草稿值（而非已保存值）发起连通性探测", async () => {
    const user = userEvent.setup();
    const { testVendorConnectivity } = renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const baseUrl = gpt.getByLabelText("GPT base_url");
    await user.clear(baseUrl);
    await user.type(baseUrl, "https://draft.example.com");
    const apiKey = gpt.getByLabelText("GPT API Key");
    await user.clear(apiKey);
    await user.type(apiKey, "sk-draft-key");
    await user.click(gpt.getByRole("button", { name: "已选 2 项" }));
    await user.click(gpt.getByRole("checkbox", { name: "gpt-image-2.6" }));

    await user.click(gpt.getByRole("button", { name: "测试连通" }));

    await waitFor(() => {
      expect(testVendorConnectivity).toHaveBeenCalledWith("gpt", {
        baseUrl: "https://draft.example.com",
        apiKey: "sk-draft-key",
        models: ["gpt-image-2.6-flare"],
      });
    });
  });

  it("未改动 key 时探测不携带脱敏占位值", async () => {
    const user = userEvent.setup();
    const { testVendorConnectivity } = renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    await user.click(gpt.getByRole("button", { name: "测试连通" }));

    await waitFor(() => {
      expect(testVendorConnectivity).toHaveBeenCalledWith(
        "gpt",
        expect.objectContaining({ apiKey: "" }),
      );
    });
  });

  it("测试连通按钮旁说明成功后会用网关模型列表更新清单", async () => {
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    expect(
      gpt.getByText("成功后会用网关返回的模型列表更新模型清单"),
    ).toBeInTheDocument();
    expect(gpt.getByText("只测连通性，不生成图片、不计费")).toBeInTheDocument();
  });

  it("探测进行中按钮禁用并显示进行中态", async () => {
    renderDialog({
      gpt: { phase: "testing", result: null, error: null },
    });
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    const button = gpt.getByRole("button", { name: "测试中…" });
    expect(button).toBeDisabled();
  });

  it("内联展示探测结果：结论、每一步详情与缺失模型", async () => {
    renderDialog({
      gpt: {
        phase: "settled",
        error: null,
        result: {
          vendor: "gpt",
          ok: false,
          message: "连通性异常",
          steps: [
            {
              key: "models",
              label: "模型列表 /v1/models",
              status: "ok",
              detail: "网关返回 3 个模型",
            },
            {
              key: "generations",
              label: "图片端点 /v1/images/generations",
              status: "failed",
              detail: "路径不对：只填到主机根路径",
            },
          ],
          missing_models: ["gpt-image-2.6-flare"],
          available_models: ["gpt-image-2.6", "gpt-image-2.6-flare"],
          duration_ms: 42,
        },
      },
    });
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    expect(gpt.getByText("连通性异常")).toBeInTheDocument();
    expect(gpt.getByText("网关返回 3 个模型")).toBeInTheDocument();
    expect(gpt.getByText(/路径不对：只填到主机根路径/)).toBeInTheDocument();
    expect(
      gpt.getByText("网关未列出这些模型：gpt-image-2.6-flare"),
    ).toBeInTheDocument();
  });

  it("探测成功时展示连通结论", async () => {
    renderDialog({
      minimax: {
        phase: "settled",
        error: null,
        result: {
          vendor: "minimax",
          ok: true,
          message: "连通性正常",
          steps: [
            {
              key: "generations",
              label: "图片端点 /v1/images/generations",
              status: "ok",
              detail: "图片端点连通且鉴权通过（缺少 prompt 属预期，不会计费）",
            },
          ],
          missing_models: [],
          available_models: ["MiniMax-H3-Image"],
          duration_ms: 8,
        },
      },
    });
    const minimax = within(
      await screen.findByRole("region", { name: "MiniMax" }),
    );

    expect(minimax.getByText("连通性正常")).toBeInTheDocument();
    expect(minimax.queryByText(/网关未列出这些模型/)).toBeNull();
  });

  it("界面不回显 API Key 明文", async () => {
    const user = userEvent.setup();
    renderDialog();
    const region = await screen.findByRole("region", { name: "GPT" });
    const gpt = within(region);

    const apiKey = gpt.getByLabelText("GPT API Key");
    await user.clear(apiKey);
    await user.type(apiKey, "sk-super-secret");
    await user.click(gpt.getByRole("button", { name: "测试连通" }));

    await waitFor(() => {
      expect(gpt.getByText(/保存后将替换当前 key/)).toBeInTheDocument();
    });
    expect(region.textContent).not.toContain("sk-super-secret");
  });

  it("探测失败不阻断保存", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog({
      gpt: { phase: "settled", result: null, error: "API Key 无效或无权限" },
    });
    await screen.findByRole("region", { name: "GPT" });

    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalled();
    });
  });
});


