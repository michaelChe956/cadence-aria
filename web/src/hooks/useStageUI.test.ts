import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useStageUI } from "./useStageUI";

describe("useStageUI", () => {
  it("returns prepare context config", () => {
    const { result } = renderHook(() => useStageUI("prepare_context"));
    expect(result.current.showContextInput).toBe(true);
    expect(result.current.providerEditable).toBe(true);
  });

  it("returns running config", () => {
    const { result } = renderHook(() => useStageUI("running"));
    expect(result.current.showContextInput).toBe(false);
    expect(result.current.providerEditable).toBe(false);
  });

  it("returns human confirm config", () => {
    const { result } = renderHook(() => useStageUI("human_confirm"));
    expect(result.current.headerBadge).toBe("等待确认");
  });

  it("returns author confirm config", () => {
    const { result } = renderHook(() => useStageUI("author_confirm"));
    expect(result.current.headerBadge).toBe("Author 待确认");
    expect(result.current.showContextInput).toBe(false);
    expect(result.current.providerEditable).toBe(false);
  });


  it("falls back to prepare context config for unknown stages", () => {
    const { result } = renderHook(() => useStageUI("unexpected"));
    expect(result.current.providerEditable).toBe(true);
  });
});
