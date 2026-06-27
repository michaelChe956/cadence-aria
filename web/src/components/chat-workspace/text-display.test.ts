import { describe, expect, it } from "vitest";
import { decodeHtmlEntitiesForDisplay, normalizeDisplayText } from "./text-display";

describe("text-display", () => {
  it("decodes common html entities without using html injection", () => {
    expect(decodeHtmlEntitiesForDisplay("&quot;cmd&quot; &amp; &lt;safe&gt;")).toBe(
      '"cmd" & <safe>',
    );
  });

  it("formats html-entity escaped json objects", () => {
    const raw = "{&quot;required_gates&quot;:[&quot;cmd_check&quot;]}";

    expect(normalizeDisplayText(raw)).toContain('"required_gates": [');
    expect(normalizeDisplayText(raw)).not.toContain("&quot;");
  });
});
