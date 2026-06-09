import { describe, expect, it } from "vitest";
import {
  emptyWorkspaceContentCache,
  getWorkspaceContentCacheValue,
  setWorkspaceContentCacheEntry,
  workspaceContentCacheValues,
} from "./workspace-content-cache";

describe("workspace content cache", () => {
  it("stores and reads cached values", () => {
    const cache = setWorkspaceContentCacheEntry(
      emptyWorkspaceContentCache(100),
      "prompt:1",
      "abc",
      10,
    );

    expect(getWorkspaceContentCacheValue(cache, "prompt:1", 20)?.value).toBe("abc");
    expect(workspaceContentCacheValues(cache)).toEqual({ "prompt:1": "abc" });
  });

  it("evicts least recently used entries when byte budget is exceeded", () => {
    let cache = emptyWorkspaceContentCache(6);
    cache = setWorkspaceContentCacheEntry(cache, "a", "aaa", 1);
    cache = setWorkspaceContentCacheEntry(cache, "b", "bbb", 2);
    cache = getWorkspaceContentCacheValue(cache, "a", 3)?.cache ?? cache;
    cache = setWorkspaceContentCacheEntry(cache, "c", "ccc", 4);

    expect(workspaceContentCacheValues(cache)).toEqual({ a: "aaa", c: "ccc" });
  });

  it("keeps oversized latest entries by evicting older entries", () => {
    let cache = emptyWorkspaceContentCache(4);
    cache = setWorkspaceContentCacheEntry(cache, "a", "aa", 1);
    cache = setWorkspaceContentCacheEntry(cache, "large", "123456", 2);

    expect(workspaceContentCacheValues(cache)).toEqual({ large: "123456" });
    expect(cache.totalBytes).toBe(6);
  });
});
