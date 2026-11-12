import { afterEach, describe, expect, it, vi } from "vitest";
import * as imageCreateTypes from "./types/image-create";
import {
  createImageCreateSession,
  generateImage,
  getImageCreateSettings,
  imageUrl,
  listImageCreateSessions,
  testImageVendorConnectivity,
  updateImageCreateSettings,
} from "./image-create";

const vendorsSettings = {
  vendors: {
    gpt: {
      base_url: "https://images.example.com",
      api_key_masked: "sk-***",
      models: ["gpt-image-2.6-flare"],
      recommended_models: ["gpt-image-2.6-flare"],
      available: true,
    },
    minimax: {
      base_url: "",
      api_key_masked: "",
      models: [],
      recommended_models: ["MiniMax-H3-Image"],
      available: false,
    },
  },
  defaults: {
    size: "auto",
    quality: "auto",
    background: "auto",
    output_format: "png",
  },
};

describe("image create api", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("uses the image-create session endpoints and request shapes", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === "/api/image-create/sessions" && init?.method === "POST") {
        return jsonResponse({
          id: "session-1",
          provider_name: "codex",
          template: { preset: "business_flow_diagram", custom: null },
          last_provider_session_id: null,
          current_prompt: null,
          status: "active",
          created_at: "2026-08-03T00:00:00Z",
        }, 201);
      }
      if (path === "/api/image-create/settings" && init?.method === "PUT") {
        return jsonResponse(vendorsSettings);
      }
      if (path === "/api/image-create/settings") {
        return jsonResponse(vendorsSettings);
      }
      return jsonResponse([]);
    });
    vi.stubGlobal("fetch", fetchMock);

    await listImageCreateSessions();
    await createImageCreateSession({
      template: { preset: "business_flow_diagram" },
      provider_name: "codex",
    });
    const settings = await getImageCreateSettings();

    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      "/api/image-create/sessions",
      expect.objectContaining({ headers: expect.any(Object) }),
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      "/api/image-create/sessions",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          template: { preset: "business_flow_diagram" },
          provider_name: "codex",
        }),
      }),
    );
    expect(settings.vendors.gpt.base_url).toBe("https://images.example.com");
    expect(settings.vendors.minimax.available).toBe(false);
  });

  it("sends the settings update as a vendors-scoped body", async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) =>
      jsonResponse(vendorsSettings),
    );
    vi.stubGlobal("fetch", fetchMock);

    const vendorKeys = [...imageCreateTypes.IMAGE_VENDORS];

    await updateImageCreateSettings({
      vendors: {
        gpt: {
          base_url: "https://images.example.com",
          api_key_action: "retain",
        },
        minimax: {
          api_key_action: "clear",
        },
      },
      defaults: {
        size: "auto",
        quality: "auto",
        background: "auto",
        output_format: "png",
      },
    });

    const init = fetchMock.mock.calls[0][1];
    expect(fetchMock.mock.calls[0][0]).toBe("/api/image-create/settings");
    expect(init?.method).toBe("PUT");
    const body = JSON.parse(init?.body as string) as Record<string, unknown>;
    expect(Object.keys(body)).toEqual(["vendors", "defaults"]);
    expect(body).not.toHaveProperty("base_url");
    expect(body).not.toHaveProperty("api_key_action");
    const vendors = body.vendors as Record<string, unknown>;
    expect(Object.keys(vendors)).toEqual(vendorKeys);
    expect(vendors.gpt).toEqual({
      base_url: "https://images.example.com",
      api_key_action: "retain",
    });
    expect(vendors.minimax).toEqual({ api_key_action: "clear" });
  });

  it("posts the connectivity probe to the settings connectivity endpoint", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      if (
        String(input) === "/api/image-create/settings/connectivity" &&
        init?.method === "POST"
      ) {
        return jsonResponse({
          vendor: "gpt",
          ok: true,
          message: "连通性正常",
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
              status: "ok",
              detail: "图片端点连通且鉴权通过（缺少 prompt 属预期，不会计费）",
            },
          ],
          missing_models: [],
          duration_ms: 12,
        });
      }
      return jsonResponse({ code: "not_found", message: "未找到" }, 404);
    });
    vi.stubGlobal("fetch", fetchMock);

    const result = await testImageVendorConnectivity({
      vendor: "gpt",
      base_url: "https://api.openai.com",
      api_key: "",
      models: ["gpt-image-2.6"],
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0][0]).toBe(
      "/api/image-create/settings/connectivity",
    );
    const init = fetchMock.mock.calls[0][1];
    expect(init?.method).toBe("POST");
    expect(JSON.parse(init?.body as string)).toEqual({
      vendor: "gpt",
      base_url: "https://api.openai.com",
      api_key: "",
      models: ["gpt-image-2.6"],
    });
    expect(result.ok).toBe(true);
    expect(result.steps).toHaveLength(2);
    expect(result.missing_models).toEqual([]);
  });

  it("surfaces a rejected connectivity probe as an api error", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ code: "invalid_base_url", message: "base_url 无效" }, 400),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(
      testImageVendorConnectivity({
        vendor: "minimax",
        base_url: "not-a-url",
        api_key: "sk-x",
        models: [],
      }),
    ).rejects.toThrow("base_url 无效");
  });

  it("sends generate as FormData without overriding its multipart boundary", async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) =>
      jsonResponse({
        media_type: "image/png",
        image_id: "image-1",
        vendor: "gpt",
        model: "gpt-image-2.6-flare",
        reference_source: "uploaded",
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const reference = new File(["reference"], "reference.png", {
      type: "image/png",
    });

    const result = await generateImage("session/with space", {
      prompt: "draw a launch diagram",
      size: "1536x1024",
      quality: "high",
      background: "transparent",
      output_format: "png",
      model: "gpt-image-2.6-flare",
      use_previous: true,
      reference,
    });

    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(
      "/api/image-create/sessions/session%2Fwith%20space/generate",
    );
    expect(init?.method).toBe("POST");
    expect(init?.headers).toBeUndefined();
    expect(init?.body).toBeInstanceOf(FormData);
    const form = init?.body as FormData;
    expect(form.get("prompt")).toBe("draw a launch diagram");
    expect(form.get("size")).toBe("1536x1024");
    expect(form.get("quality")).toBe("high");
    expect(form.get("background")).toBe("transparent");
    expect(form.get("output_format")).toBe("png");
    expect(form.get("model")).toBe("gpt-image-2.6-flare");
    expect(form.get("use_previous")).toBe("true");
    expect(form.has("input_fidelity")).toBe(false);
    expect(form.get("reference")).toBe(reference);
    expect(result).toEqual({
      media_type: "image/png",
      image_id: "image-1",
      vendor: "gpt",
      model: "gpt-image-2.6-flare",
      reference_source: "uploaded",
    });
  });

  it("rejects an empty model before sending the request", async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) =>
      jsonResponse({}),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(
      generateImage("session-1", {
        prompt: "draw from text",
        size: "auto",
        quality: "auto",
        background: "auto",
        output_format: "png",
        model: "  ",
        use_previous: false,
        reference: null,
      }),
    ).rejects.toThrow("请先选择图片生成模型");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("omits the reference image but still sends model and use_previous", async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) =>
      jsonResponse({
        media_type: "image/png",
        image_id: "image-1",
        vendor: "minimax",
        model: "MiniMax-H3-Image",
        reference_source: "none",
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await generateImage("session-1", {
      prompt: "draw from text",
      size: "auto",
      quality: "auto",
      background: "auto",
      output_format: "png",
      model: "MiniMax-H3-Image",
      use_previous: false,
      reference: null,
    });

    const form = fetchMock.mock.calls[0][1]?.body as FormData;
    expect(form.has("reference")).toBe(false);
    expect(form.has("input_fidelity")).toBe(false);
    expect(form.get("model")).toBe("MiniMax-H3-Image");
    expect(form.get("use_previous")).toBe("false");
  });

  it("builds encoded session image endpoint URLs", () => {
    expect(imageUrl("session/with space", "image/id & value")).toBe(
      "/api/image-create/sessions/session%2Fwith%20space/images/image%2Fid%20%26%20value",
    );
  });

  it.each([
    [{ error: "图片生成服务未配置" }, "图片生成服务未配置"],
    [{ error: { message: "参考图格式不受支持" } }, "参考图格式不受支持"],
  ])("extracts image-create backend errors from %j", async (body, message) => {
    vi.stubGlobal("fetch", vi.fn(async () => jsonResponse(body, 400)));

    await expect(listImageCreateSessions()).rejects.toThrow(message);
  });
});

function jsonResponse(body: unknown, status = 200) {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    }),
  );
}
