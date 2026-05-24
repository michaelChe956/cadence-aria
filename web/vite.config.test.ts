import { describe, expect, it } from "vitest";
import { devServerProxy } from "./dev-server-proxy";

describe("vite config", () => {
  it("proxies coding workspace websocket traffic to the backend", () => {
    expect(devServerProxy["/ws"]).toMatchObject({
      target: "http://127.0.0.1:4317",
      ws: true,
    });
  });
});
