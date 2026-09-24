// F-50 裁决 6（协议错误语言）：面向用户的错误呈现统一走本 copy 单一事实源——
// 中文主显 lead + 可选中文正文 body；错误码/英文原文只作为诊断信息由各错误面
// 以 mono/折叠详情承载，不再把长英文当作主文案。
import { describe, expect, it } from "vitest";
import {
  HARD_ERROR_DRAWER_REFERENCE_NOTE,
  PROTOCOL_ERROR_DETAILS_LABEL,
  PROTOCOL_ERROR_NO_RETRY_NOTE,
  protocolErrorCopy,
} from "./protocol-error-copy";

describe("protocol-error-copy (F-50 裁决 5/6/7)", () => {
  it("maps the stale driver lease code to a Chinese lead and body", () => {
    expect(protocolErrorCopy("STALE_DRIVER_LEASE")).toEqual({
      lead: "连接租约已失效",
      body: "本连接已失去写入租约，当前操作未提交。",
    });
  });

  it("maps the observer write rejection code without inventing a body", () => {
    expect(protocolErrorCopy("OBSERVER_WRITE_REJECTED")).toEqual({
      lead: "本连接为观察者连接，写操作已被拒收",
      body: null,
    });
  });

  it("falls back to a neutral lead for unknown codes (fail-closed，不猜语义)", () => {
    expect(protocolErrorCopy("SOME_FUTURE_CODE")).toEqual({
      lead: "操作被拒绝",
      body: null,
    });
  });

  it("keeps the shared presentation labels stable", () => {
    expect(PROTOCOL_ERROR_DETAILS_LABEL).toBe("查看诊断详情");
    expect(PROTOCOL_ERROR_NO_RETRY_NOTE).toBe("该错误不支持安全重试");
    expect(HARD_ERROR_DRAWER_REFERENCE_NOTE).toBe("处理入口在待处理抽屉");
  });
});
