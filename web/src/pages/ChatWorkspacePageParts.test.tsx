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

  it("未启用 review 时仍携带已选 reviewer 且 rounds 为 0（provisional 快照）", () => {
    const snapshot = providerConfigFor(
      { author: "claude_code", reviewer: "codex" },
      false,
      3,
    );

    expect(snapshot.reviewer).toBe("codex");
    expect(snapshot.review_rounds).toBe(0);
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
