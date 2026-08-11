import { describe, expect, it } from "vitest";
import { providerName } from "./workspace-ws-message-handler";

describe("workspace websocket provider parser", () => {
  it("accepts kimi_code as a workspace provider", () => {
    expect(providerName("kimi_code")).toBe("kimi_code");
  });

  it("rejects unknown provider names", () => {
    expect(providerName("unknown_provider")).toBeNull();
  });
});
