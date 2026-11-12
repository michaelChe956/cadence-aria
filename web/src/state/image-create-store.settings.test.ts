import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ConnectivityProbeResult,
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
  testImageVendorConnectivity: vi.fn(),
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

  it("picks the first recommended model of the vendor when it is in the list", async () => {
    api.getImageCreateSettings.mockResolvedValue(
      maskedSettings({
        gpt: { models: ["gpt-image-2.6", "gpt-image-2.6-flare"] },
      }),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().loadSettings();

    // 推荐清单的首个 id 命中，即使它不是清单首项。
    expect(useImageCreateStore.getState().params.model).toBe(
      "gpt-image-2.6-flare",
    );
  });

  it("falls back to the first id containing image when no recommended model matches", async () => {
    api.getImageCreateSettings.mockResolvedValue(
      maskedSettings({
        gpt: {
          models: ["text-embedding-3", "GPT-IMAGE-9", "gpt-image-2.6"],
          recommended_models: ["gpt-image-9"],
        },
      }),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().loadSettings();

    // 含 "image" 的判断忽略大小写。
    expect(useImageCreateStore.getState().params.model).toBe("GPT-IMAGE-9");
  });

  it("falls back to the first id when neither recommended nor image matches", async () => {
    api.getImageCreateSettings.mockResolvedValue(
      maskedSettings({
        gpt: {
          models: ["text-embedding-3", "claude-3"],
          recommended_models: ["gpt-image-2.6"],
        },
      }),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().loadSettings();

    expect(useImageCreateStore.getState().params.model).toBe("text-embedding-3");
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

describe("image create store 连通性探测", () => {
  it("probes with the given draft values and keeps the result per vendor", async () => {
    api.testImageVendorConnectivity.mockResolvedValue(probeResult());
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().testVendorConnectivity("gpt", {
      baseUrl: "https://draft.example.com",
      apiKey: "sk-draft-key",
      models: ["gpt-image-2.6", "gpt-image-2.6-flare"],
    });

    expect(api.testImageVendorConnectivity).toHaveBeenCalledWith({
      vendor: "gpt",
      base_url: "https://draft.example.com",
      api_key: "sk-draft-key",
      models: ["gpt-image-2.6", "gpt-image-2.6-flare"],
    });
    const state = useImageCreateStore.getState().connectivity.gpt;
    expect(state.phase).toBe("settled");
    expect(state.result?.ok).toBe(false);
    expect(state.result?.missing_models).toEqual(["gpt-image-2.6-flare"]);
    expect(state.error).toBeNull();
    // 未探测的那家不受影响。
    expect(useImageCreateStore.getState().connectivity.minimax.phase).toBe(
      "idle",
    );
  });

  it("marks the vendor as probing while the request is in flight", async () => {
    const deferred = createDeferred<ConnectivityProbeResult>();
    api.testImageVendorConnectivity.mockImplementation(() => deferred.promise);
    const { useImageCreateStore } = await import("./image-create-store");

    const pending = useImageCreateStore
      .getState()
      .testVendorConnectivity("minimax", {
        baseUrl: "https://api.minimax.io",
        apiKey: "",
        models: [],
      });
    expect(useImageCreateStore.getState().connectivity.minimax.phase).toBe(
      "testing",
    );

    deferred.resolve(probeResult());
    await pending;
    expect(useImageCreateStore.getState().connectivity.minimax.phase).toBe(
      "settled",
    );
  });

  it("rejects an invalid base url locally without probing", async () => {
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().testVendorConnectivity("gpt", {
      baseUrl: "http://evil.example.com",
      apiKey: "",
      models: [],
    });

    expect(api.testImageVendorConnectivity).not.toHaveBeenCalled();
    const state = useImageCreateStore.getState().connectivity.gpt;
    expect(state.phase).toBe("settled");
    expect(state.error).toContain("base_url");
  });

  it("asks for a base url before probing", async () => {
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().testVendorConnectivity("gpt", {
      baseUrl: "   ",
      apiKey: "",
      models: [],
    });

    expect(api.testImageVendorConnectivity).not.toHaveBeenCalled();
    expect(
      useImageCreateStore.getState().connectivity.gpt.error,
    ).toContain("Base URL");
  });

  it("keeps the api key out of the probe error message", async () => {
    api.testImageVendorConnectivity.mockRejectedValue(
      new Error("请求失败：sk-super-secret 无效"),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().testVendorConnectivity("gpt", {
      baseUrl: "https://api.openai.com",
      apiKey: "sk-super-secret",
      models: [],
    });

    const state = useImageCreateStore.getState().connectivity.gpt;
    expect(state.error).toContain("请求失败");
    expect(state.error).not.toContain("sk-super-secret");
    expect(JSON.stringify(state)).not.toContain("sk-super-secret");
  });
});

describe("image create store 模型候选池", () => {
  async function probeGpt(result: ConnectivityProbeResult = probeResult()) {
    api.testImageVendorConnectivity.mockResolvedValue(result);
    const { useImageCreateStore } = await import("./image-create-store");
    await useImageCreateStore.getState().testVendorConnectivity("gpt", {
      baseUrl: "https://api.openai.com",
      apiKey: "",
      models: [],
    });
    return useImageCreateStore;
  }

  it("keeps the gateway model list per vendor after a probe", async () => {
    const useImageCreateStore = await probeGpt();

    expect(useImageCreateStore.getState().modelPools.gpt.gateway).toEqual([
      "gpt-image-2.6",
      "gpt-image-2.6-flare",
      "gpt-image-3",
    ]);
    // 未探测的那家没有网关列表。
    expect(useImageCreateStore.getState().modelPools.minimax.gateway).toEqual(
      [],
    );
  });

  it("refreshes the candidate pool with the gateway list but keeps the checked list unchanged", async () => {
    const useImageCreateStore = await probeGpt();

    const refresh = useImageCreateStore
      .getState()
      .refreshModelCandidates("gpt", ["gpt-image-2.6-flare", "custom-x"], []);

    // 候选池 = 网关模型（网关顺序在前）∪ 草稿清单里已有的。
    expect(refresh.candidates).toEqual([
      "gpt-image-2.6",
      "gpt-image-2.6-flare",
      "gpt-image-3",
      "custom-x",
    ]);
    expect(useImageCreateStore.getState().modelPools.gpt.candidates).toEqual(
      refresh.candidates,
    );
    // 勾选集合 = 草稿清单原样：探测不新增勾选，也不取消勾选。
    expect(refresh.selected).toEqual(["gpt-image-2.6-flare", "custom-x"]);
  });

  it("checks every gateway model when the current list is empty", async () => {
    const useImageCreateStore = await probeGpt();

    const refresh = useImageCreateStore
      .getState()
      .refreshModelCandidates("gpt", [], []);

    expect(refresh.selected).toEqual([
      "gpt-image-2.6",
      "gpt-image-2.6-flare",
      "gpt-image-3",
    ]);
    expect(useImageCreateStore.getState().modelPools.gpt.candidates).toEqual(
      refresh.selected,
    );
  });

  it("skips the models already used by the other vendor", async () => {
    const useImageCreateStore = await probeGpt();

    const refresh = useImageCreateStore
      .getState()
      .refreshModelCandidates("gpt", [], ["gpt-image-2.6"]);

    expect(refresh.skipped).toEqual(["gpt-image-2.6"]);
    expect(refresh.candidates).not.toContain("gpt-image-2.6");
    expect(refresh.selected).toEqual(["gpt-image-2.6-flare", "gpt-image-3"]);
  });

  it("keeps only the first 256 gateway models when auto-selecting", async () => {
    const many = Array.from({ length: 300 }, (_, index) => `m-${index}`);
    const useImageCreateStore = await probeGpt({
      ...probeResult(),
      available_models: many,
    });

    const refresh = useImageCreateStore
      .getState()
      .refreshModelCandidates("gpt", [], []);

    expect(refresh.selected).toHaveLength(256);
    expect(refresh.selected[0]).toBe("m-0");
    expect(refresh.truncated).toEqual({ total: 300, kept: 256 });
  });

  it("keeps the candidate pool and the checked list when the gateway list is empty", async () => {
    const useImageCreateStore = await probeGpt();
    useImageCreateStore.getState().setVendorModels("minimax", ["m-a"]);

    const refresh = useImageCreateStore
      .getState()
      .refreshModelCandidates("minimax", ["m-a"], []);

    expect(refresh.candidates).toEqual(["m-a"]);
    expect(refresh.selected).toEqual(["m-a"]);
    expect(refresh.skipped).toEqual([]);
    expect(refresh.truncated).toBeNull();
  });

  it("merges extra model ids into the candidate pool and checks them", async () => {
    const useImageCreateStore = await probeGpt();
    useImageCreateStore
      .getState()
      .refreshModelCandidates("gpt", ["gpt-image-2.6"], []);

    const selected = useImageCreateStore
      .getState()
      .mergeVendorModels("gpt", ["gpt-image-2.6"], ["gpt-image-4", "gpt-image-3"]);

    expect(useImageCreateStore.getState().modelPools.gpt.candidates).toEqual([
      "gpt-image-2.6",
      "gpt-image-2.6-flare",
      "gpt-image-3",
      "gpt-image-4",
    ]);
    // 勾选集合 = 已有清单 ∪ 并入的 id；池里没被勾选的项不算。
    expect(selected).toEqual([
      "gpt-image-2.6",
      "gpt-image-3",
      "gpt-image-4",
    ]);
  });

  it("replaces the candidate pool when loading the saved list", async () => {
    const useImageCreateStore = await probeGpt();

    const models = useImageCreateStore
      .getState()
      .setVendorModels("gpt", ["saved-a", "saved-a", " saved-b "]);

    expect(models).toEqual(["saved-a", "saved-b"]);
    expect(useImageCreateStore.getState().modelPools.gpt.candidates).toEqual([
      "saved-a",
      "saved-b",
    ]);
  });
});

/**
 * 一次性 deferred：项目 tsconfig 的 lib 是 ES2022，没有 Promise.withResolvers，
 * 这里手写等价物，用来控制探测请求的完成时机。
 */
function createDeferred<T>() {
  let resolve: (value: T) => void = () => {};
  let reject: (reason: unknown) => void = () => {};
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** 构造一份探测结果（网关未列出其中一个模型）。 */
function probeResult(): ConnectivityProbeResult {
  return {
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
    available_models: ["gpt-image-2.6", "gpt-image-2.6-flare", "gpt-image-3"],
    duration_ms: 42,
  };
}

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
        recommended_models: ["gpt-image-2.6-flare", "gpt-image-2.6"],
        available: true,
        ...overrides.gpt,
      }),
      minimax: vendor({
        recommended_models: ["MiniMax-H3-Image"],
        ...overrides.minimax,
      }),
    },
    defaults: {
      size: "auto",
      quality: "auto",
      background: "auto",
      output_format: "png",
    },
  };
}
