import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createImageCreateSession,
  generateImage,
  getImageCreateSettings,
  listImageCreateSessions,
  updateImageCreateSettings,
} from "./image-create";

describe("image create api", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("uses the image-create JSON endpoints and request shapes", async () => {
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
        return jsonResponse({
          base_url: "https://images.example.com",
          api_key_masked: "sk-***",
          defaults: {
            size: "auto",
            quality: "auto",
            background: "auto",
            output_format: "png",
          },
        });
      }
      if (path === "/api/image-create/settings") {
        return jsonResponse({
          base_url: "",
          api_key_masked: "",
          defaults: {
            size: "auto",
            quality: "auto",
            background: "auto",
            output_format: "png",
          },
        });
      }
      return jsonResponse([]);
    });
    vi.stubGlobal("fetch", fetchMock);

    await listImageCreateSessions();
    await createImageCreateSession({
      template: { preset: "business_flow_diagram" },
      provider_name: "codex",
    });
    await getImageCreateSettings();
    await updateImageCreateSettings({
      base_url: "https://images.example.com",
      api_key_action: "retain",
      defaults: null,
    });

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
    expect(fetchMock).toHaveBeenNthCalledWith(
      4,
      "/api/image-create/settings",
      expect.objectContaining({ method: "PUT" }),
    );
  });

  it("sends generate as FormData without overriding its multipart boundary", async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) =>
      jsonResponse({ media_type: "image/png", b64: "aW1hZ2U=" }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const reference = new File(["reference"], "reference.png", {
      type: "image/png",
    });

    await generateImage("session/with space", {
      prompt: "draw a launch diagram",
      size: "1536x1024",
      quality: "high",
      background: "transparent",
      output_format: "png",
      input_fidelity: "high",
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
    expect(form.get("input_fidelity")).toBe("high");
    expect(form.get("reference")).toBe(reference);
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
