import { describe, expect, it } from "vitest";
import { providerConfigFor } from "./ChatWorkspacePageParts";

describe("providerConfigFor", () => {
  it("保留 Kimi Code 选择、不回退且保留 Supervised 权限模式", () => {
    const snapshot = providerConfigFor(
      { author: "kimi_code", reviewer: "kimi_code" },
      true,
      1,
      { author: "supervised", reviewer: "supervised" },
    );

    expect(snapshot).toMatchObject({
      author: "kimi_code",
      reviewer: "kimi_code",
      permission_modes: { author: "supervised", reviewer: "supervised" },
    });
  });

  it("保留 pi 选择不回退并序列化权限模式", () => {
    const snapshot = providerConfigFor(
      { author: "pi", reviewer: "codex" },
      true,
      1,
      { author: "auto", reviewer: "auto" },
    );

    expect(snapshot.author).toBe("pi");
    expect(snapshot.permission_modes?.author).toBe("auto");
  });
});
