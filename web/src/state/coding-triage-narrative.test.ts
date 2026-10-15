// P2-d（REQ-CRO-09）红测：分诊卡「三段大白话」纯函数。
// 契约：cadence/plans/2026-10-15_实施计划_批量1主流程_v1.0.md §2 波 3 P2-d ⑥
// （有/无 summary、N>1 finding、无 delivery）＋设计块 2 §3.4——
// 措辞写死在本模块，triage_summary 缺失时 problem 回落 gate.description。
import { describe, expect, it } from "vitest";
import { buildTriageNarrative } from "./coding-triage-narrative";

describe("buildTriageNarrative（P2-d 分诊卡三段大白话）", () => {
  it("有 triage_summary：problem/did/unverified 取自结构化字段", () => {
    const narrative = buildTriageNarrative(
      {
        problem: "数据库连接被拒绝，迁移无法执行",
        findings: [
          {
            reason_code: "environment_port_blocked",
            message: "数据库连接被拒绝，迁移无法执行",
          },
        ],
        did: {
          changed_files: ["src/main.rs", "src/lib.rs", "src/api.rs", "tests/it.rs"],
          commit_sha: "a1b2c3d4e5f60718",
          verification_exit_code: null,
          environment_note: null,
        },
        unverified: ["cargo test 未运行", "迁移脚本未在真实库上验证"],
      },
      "回落描述",
    );

    expect(narrative.problem).toBe("数据库连接被拒绝，迁移无法执行");
    expect(narrative.did).not.toBeNull();
    expect(narrative.did?.headline).toContain("改了 4 个文件");
    expect(narrative.did?.headline).toContain("已提交");
    expect(narrative.did?.headline).toContain("a1b2c3d");
    expect(narrative.did?.changedFiles).toEqual([
      "src/main.rs",
      "src/lib.rs",
      "src/api.rs",
      "tests/it.rs",
    ]);
    expect(narrative.unverified).toEqual([
      "cargo test 未运行",
      "迁移脚本未在真实库上验证",
    ]);
  });

  it("无 triage_summary（旧门 JSON 反序列化为 null/缺字段）：problem 回落 description，did/unverified 为空", () => {
    for (const summary of [null, undefined]) {
      const narrative = buildTriageNarrative(
        summary,
        "Coder 输出需要人工分诊，流程已暂停",
      );
      expect(narrative.problem).toBe("Coder 输出需要人工分诊，流程已暂停");
      expect(narrative.did).toBeNull();
      expect(narrative.unverified).toEqual([]);
      expect(narrative.findings).toEqual([]);
    }
  });

  it("N>1 findings：逐条保留 reason_code 与 message（技术详情折叠区整查）", () => {
    const narrative = buildTriageNarrative(
      {
        problem: "首条 finding 的问题",
        findings: [
          { reason_code: "environment_port_blocked", message: "首条 finding 的问题" },
          { reason_code: "tool_missing", message: "缺少 psql 客户端" },
          { reason_code: "network_unreachable", message: "网关不可达" },
        ],
        did: null,
        unverified: [],
      },
      "描述",
    );

    expect(narrative.problem).toBe("首条 finding 的问题");
    expect(narrative.findings).toEqual([
      { reasonCode: "environment_port_blocked", message: "首条 finding 的问题" },
      { reasonCode: "tool_missing", message: "缺少 psql 客户端" },
      { reasonCode: "network_unreachable", message: "网关不可达" },
    ]);
  });

  it("无 delivery（did=null）：整块不显示（narrative.did 为 null）", () => {
    const narrative = buildTriageNarrative(
      { problem: "p", findings: [], did: null, unverified: ["u"] },
      "d",
    );
    expect(narrative.did).toBeNull();
    expect(narrative.unverified).toEqual(["u"]);
  });

  it("未提交（无 commit_sha）：headline 标注「未提交」而非「已提交」", () => {
    const narrative = buildTriageNarrative(
      {
        problem: "p",
        findings: [],
        did: {
          changed_files: ["a.rs"],
          commit_sha: null,
          verification_exit_code: 1,
          environment_note: null,
        },
        unverified: [],
      },
      "d",
    );
    expect(narrative.did?.headline).toContain("改了 1 个文件");
    expect(narrative.did?.headline).toContain("未提交");
    expect(narrative.did?.headline).not.toContain("已提交");
  });
});
