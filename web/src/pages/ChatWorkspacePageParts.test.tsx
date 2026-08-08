import { describe, expect, it } from "vitest";
import { providerConfigFor } from "./ChatWorkspacePageParts";

describe("providerConfigFor", () => {
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
