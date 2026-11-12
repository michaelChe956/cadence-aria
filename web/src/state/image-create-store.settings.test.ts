import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ImageVendor,
  MaskedSettings,
  MaskedVendorSettings,
} from "../api/types/image-create";

const api = vi.hoisted(() => ({
  listImageCreateSessions: vi.fn(),
  getImageCreateSession: vi.fn(),
  createImageCreateSession: vi.fn(),
  deleteImageCreateSession: vi.fn(),
  generateImage: vi.fn(),
  getImageCreateSettings: vi.fn(),
  updateImageCreateSettings: vi.fn(),
  imageCreateChatWebSocketUrl: vi.fn(
    (sessionId: string) =>
      `ws://localhost:3000/api/image-create/sessions/${sessionId}/chat`,
  ),
  imageUrl: vi.fn(
    (sessionId: string, imageId: string) =>
      `/api/image-create/sessions/${encodeURIComponent(sessionId)}/images/${encodeURIComponent(imageId)}`,
  ),
}));

vi.mock("../api/image-create", () => api);

beforeEach(() => {
  vi.resetModules();
  vi.clearAllMocks();
});

describe("image create store 模型选择与设置", () => {
  it("starts without a model and keeps the previous-image switch enabled", async () => {
    const { useImageCreateStore } = await import("./image-create-store");

    expect(useImageCreateStore.getState().params.model).toBe("");
    expect(useImageCreateStore.getState().params.use_previous).toBe(true);
  });

  it("selects the first model of the first available vendor after loading settings", async () => {
    api.getImageCreateSettings.mockResolvedValue(
      maskedSettings({
        gpt: { models: ["gpt-image-2.6-flare", "gpt-image-2.6"] },
        minimax: {
          base_url: "https://minimax.example.com/v1",
          api_key_masked: "mm-****5678",
          models: ["MiniMax-H3-Image"],
          available: true,
        },
      }),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().loadSettings();

    expect(useImageCreateStore.getState().params.model).toBe(
      "gpt-image-2.6-flare",
    );
  });

  it("selects the MiniMax model when MiniMax is the only configured vendor", async () => {
    api.getImageCreateSettings.mockResolvedValue(
      maskedSettings({
        gpt: { base_url: "", api_key_masked: "", models: [], available: false },
        minimax: {
          base_url: "https://minimax.example.com/v1",
          api_key_masked: "mm-****5678",
          models: ["MiniMax-H3-Image"],
          available: true,
        },
      }),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().loadSettings();

    expect(useImageCreateStore.getState().params.model).toBe("MiniMax-H3-Image");
  });

  it("keeps the model empty when no vendor is configured", async () => {
    api.getImageCreateSettings.mockResolvedValue(
      maskedSettings({
        gpt: { base_url: "", api_key_masked: "", models: [], available: false },
      }),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().loadSettings();

    expect(useImageCreateStore.getState().params.model).toBe("");
  });

  it("keeps the selected model while a configured vendor still offers it", async () => {
    const settings = maskedSettings({
      gpt: { models: ["gpt-image-2.6"] },
    });
    api.getImageCreateSettings.mockResolvedValue(settings);
    api.updateImageCreateSettings.mockResolvedValue(settings);
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().loadSettings();
    useImageCreateStore.getState().setParams({ model: "gpt-image-2.6" });
    await useImageCreateStore
      .getState()
      .saveSettings({ vendors: { gpt: { api_key_action: "retain" } } });

    expect(useImageCreateStore.getState().params.model).toBe("gpt-image-2.6");
  });

  it("resets the selected model to the first available model when no available vendor offers it", async () => {
    api.getImageCreateSettings.mockResolvedValue(
      maskedSettings({
        gpt: { models: ["gpt-image-2.6-flare", "gpt-image-2.6"] },
        minimax: { models: ["MiniMax-H3-Image"] },
      }),
    );
    api.updateImageCreateSettings.mockResolvedValue(
      maskedSettings({
        gpt: { models: ["gpt-image-2.6-flare", "gpt-image-2.6"] },
        minimax: { models: ["MiniMax-H3-Image"] },
      }),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().loadSettings();
    // MiniMax 未配置凭据，其模型不进入可选清单。
    useImageCreateStore.getState().setParams({ model: "MiniMax-H3-Image" });
    await useImageCreateStore
      .getState()
      .saveSettings({ vendors: { gpt: { api_key_action: "retain" } } });

    expect(useImageCreateStore.getState().params.model).toBe(
      "gpt-image-2.6-flare",
    );
  });

  it("empties the selected model when no vendor stays configured", async () => {
    api.getImageCreateSettings.mockResolvedValue(
      maskedSettings({ gpt: { models: ["gpt-image-2.6-flare"] } }),
    );
    api.updateImageCreateSettings.mockResolvedValue(
      maskedSettings({
        gpt: { base_url: "", api_key_masked: "", models: [], available: false },
      }),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().loadSettings();
    await useImageCreateStore
      .getState()
      .saveSettings({ vendors: { gpt: { api_key_action: "clear" } } });

    expect(useImageCreateStore.getState().params.model).toBe("");
  });

  it("saveSettings locally rejects a non-https base_url without calling the API", async () => {
    const { useImageCreateStore } = await import("./image-create-store");

    await expect(
      useImageCreateStore.getState().saveSettings({
        vendors: {
          minimax: { base_url: "http://evil.example.com", api_key_action: "retain" },
        },
      }),
    ).rejects.toThrow("base_url 必须使用 HTTPS，或使用 localhost/loopback IP");
    expect(api.updateImageCreateSettings).not.toHaveBeenCalled();
  });

  it("saveSettings skips local base_url validation for empty values and absent vendors", async () => {
    api.updateImageCreateSettings.mockResolvedValue(maskedSettings());
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore
      .getState()
      .saveSettings({ vendors: { minimax: { base_url: "", api_key_action: "retain" } } });
    await useImageCreateStore.getState().saveSettings({});

    expect(api.updateImageCreateSettings).toHaveBeenCalledTimes(2);
  });
});

/** 构造脱敏设置：默认只有 GPT 家配置了凭据。 */
function maskedSettings(
  overrides: Partial<Record<ImageVendor, Partial<MaskedVendorSettings>>> = {},
): MaskedSettings {
  const vendor = (base: Partial<MaskedVendorSettings>): MaskedVendorSettings => ({
    base_url: "",
    api_key_masked: "",
    models: [],
    recommended_models: [],
    available: false,
    ...base,
  });
  return {
    vendors: {
      gpt: vendor({
        base_url: "https://gpt.example.com/v1",
        api_key_masked: "sk-****t123",
        models: ["gpt-image-2.6"],
        recommended_models: ["gpt-image-2.6"],
        available: true,
        ...overrides.gpt,
      }),
      minimax: vendor(overrides.minimax ?? {}),
    },
    defaults: {
      size: "auto",
      quality: "auto",
      background: "auto",
      output_format: "png",
    },
  };
}
