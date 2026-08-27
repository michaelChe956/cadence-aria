import { describe, expect, it } from "vitest";
import { CHANGELOG, CURRENT_VERSION, recentEntries } from "./changelog";

function versionParts(version: string): number[] {
  return version.split(".").map(Number);
}

function compareVersions(left: string, right: string): number {
  const leftParts = versionParts(left);
  const rightParts = versionParts(right);

  for (let index = 0; index < Math.max(leftParts.length, rightParts.length); index += 1) {
    const difference = (leftParts[index] ?? 0) - (rightParts[index] ?? 0);
    if (difference !== 0) {
      return difference;
    }
  }

  return 0;
}

describe("CHANGELOG", () => {
  it("条目按版本从新到旧排列", () => {
    for (let index = 1; index < CHANGELOG.length; index += 1) {
      expect(compareVersions(CHANGELOG[index - 1].version, CHANGELOG[index].version)).toBeGreaterThan(0);
    }
  });

  it("从当前版本起展示至多四条并排除未发布的预备条目", () => {
    const entries = recentEntries(CURRENT_VERSION);

    expect(entries).toHaveLength(4);
    expect(entries.map((entry) => entry.version)).toEqual(["0.0.9", "0.0.8", "0.0.7", "0.0.6"]);
    expect(entries.some((entry) => entry.version === "0.0.5")).toBe(false);
  });

  it("找不到当前版本时返回空数组", () => {
    expect(recentEntries("9.9.9")).toEqual([]);
  });
});
