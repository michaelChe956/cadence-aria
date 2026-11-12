import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ConnectivityProbeResult,
  MaskedSettings,
} from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";
import { SettingsDialog } from "./SettingsDialog";

/** 连通性探测的 API 层替身：让真实的 store 动作跑完整链路。 */
const probeApi = vi.hoisted(() => vi.fn());
vi.mock("../../api/image-create", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api/image-create")>();
  return { ...actual, testImageVendorConnectivity: probeApi };
});

const originalState = useImageCreateStore.getState();

const settings: MaskedSettings = {
  vendors: {
    gpt: {
      base_url: "https://api.openai.com/v1",
      api_key_masked: "sk-****1234",
      models: ["gpt-image-2.6", "gpt-image-2.6-flare"],
      recommended_models: ["gpt-image-2.6-flare", "gpt-image-2.6"],
      available: true,
    },
    minimax: {
      base_url: "https://api.minimax.chat/v1",
      api_key_masked: "mm-****5678",
      models: ["MiniMax-H3-Image"],
      recommended_models: ["MiniMax-H3-Image"],
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

/** 渲染弹窗：走真实的 store 探测动作，只替换 API 层。 */
function renderDialog(loaded: MaskedSettings = settings) {
  const loadSettings = vi.fn(async () => {
    useImageCreateStore.setState({ settings: loaded });
  });
  const saveSettings = vi.fn().mockResolvedValue(undefined);
  useImageCreateStore.setState({
    loadSettings,
    saveSettings,
    testVendorConnectivity: originalState.testVendorConnectivity,
    connectivity: {
      gpt: { phase: "idle", result: null, error: null },
      minimax: { phase: "idle", result: null, error: null },
    },
    modelPools: {
      gpt: { gateway: [], candidates: [] },
      minimax: { gateway: [], candidates: [] },
    },
  });

  render(<SettingsDialog onClose={vi.fn()} />);

  return { loadSettings, saveSettings };
}

/** 构造一份探测结果：网关返回给定的模型列表。 */
function probeResult(
  availableModels: string[] = [
    "gpt-image-2.6",
    "gpt-image-2.6-flare",
    "gpt-image-3",
  ],
): ConnectivityProbeResult {
  return {
    vendor: "gpt",
    ok: true,
    message: "连通性正常",
    steps: [
      {
        key: "models",
        label: "模型列表 /v1/models",
        status: "ok",
        detail: `网关返回 ${availableModels.length} 个模型`,
      },
    ],
    missing_models: [],
    available_models: availableModels,
    duration_ms: 10,
  };
}

/** 只有 MiniMax 一家清单为空：用来验证「草稿为空时全选网关模型」。 */
function minimaxWithoutModels(): MaskedSettings {
  return {
    ...settings,
    vendors: {
      ...settings.vendors,
      minimax: { ...settings.vendors.minimax, models: [] },
    },
  };
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
  probeApi.mockReset();
});

describe("SettingsDialog 模型清单与探测联动", () => {
  it("探测成功后草稿清单替换为网关返回的模型", async () => {
    const user = userEvent.setup();
    probeApi.mockResolvedValue(
      probeResult(["gpt-image-2.6", "gpt-image-2.6-flare", "gpt-image-3"]),
    );
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    await user.click(gpt.getByRole("button", { name: "测试连通" }));

    await waitFor(() => {
      expect(gpt.getByRole("button", { name: "已选 3 项" })).toBeInTheDocument();
    });
    await user.click(gpt.getByRole("button", { name: "已选 3 项" }));
    expect(gpt.getByRole("checkbox", { name: "gpt-image-3" })).toBeChecked();
  });

  it("取消勾选后再次探测会重新勾上（已知取舍）", async () => {
    const user = userEvent.setup();
    probeApi.mockResolvedValue(
      probeResult(["gpt-image-2.6", "gpt-image-2.6-flare"]),
    );
    renderDialog();
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    // 先取消一个模型的勾选。
    await user.click(gpt.getByRole("button", { name: "已选 2 项" }));
    await user.click(gpt.getByRole("checkbox", { name: "gpt-image-2.6-flare" }));
    expect(gpt.getByRole("button", { name: "已选 1 项" })).toBeInTheDocument();

    // 再次探测：网关列表把它重新勾上（已知取舍，界面有说明文案）。
    await user.click(gpt.getByRole("button", { name: "测试连通" }));

    await waitFor(() => {
      expect(gpt.getByRole("button", { name: "已选 2 项" })).toBeInTheDocument();
    });
  });

  it("草稿清单为空时探测后全选网关模型", async () => {
    const user = userEvent.setup();
    probeApi.mockResolvedValue(probeResult(["m-1", "m-2", "m-3"]));
    renderDialog(minimaxWithoutModels());
    const minimax = within(
      await screen.findByRole("region", { name: "MiniMax" }),
    );

    await user.click(minimax.getByRole("button", { name: "测试连通" }));

    await waitFor(() => {
      expect(minimax.getByRole("button", { name: "已选 3 项" })).toBeInTheDocument();
    });
  });

  it("刷新时跳过已被另一家勾选的模型并提示", async () => {
    const user = userEvent.setup();
    probeApi.mockResolvedValue(probeResult(["gpt-image-2.6", "gpt-image-3"]));
    renderDialog({
      ...settings,
      vendors: {
        ...settings.vendors,
        minimax: { ...settings.vendors.minimax, models: ["gpt-image-2.6"] },
      },
    });
    const gpt = within(await screen.findByRole("region", { name: "GPT" }));

    await user.click(gpt.getByRole("button", { name: "测试连通" }));

    expect(
      await gpt.findByText("以下模型已被 MiniMax 使用，已跳过：gpt-image-2.6"),
    ).toBeInTheDocument();
  });

  it("网关模型超过单家上限时只保留前 256 个并提示", async () => {
    const user = userEvent.setup();
    probeApi.mockResolvedValue(
      probeResult(Array.from({ length: 300 }, (_, index) => `m-${index}`)),
    );
    renderDialog(minimaxWithoutModels());
    const minimax = within(
      await screen.findByRole("region", { name: "MiniMax" }),
    );

    await user.click(minimax.getByRole("button", { name: "测试连通" }));

    expect(
      await minimax.findByText(
        "网关返回 300 个模型，超过单家上限 256，已只保留前 256 个，请自行精简",
      ),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(
        minimax.getByRole("button", { name: "已选 256 项" }),
      ).toBeInTheDocument();
    });
  });
});
