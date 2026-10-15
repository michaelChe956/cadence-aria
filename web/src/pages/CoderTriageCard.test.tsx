// P2-d（REQ-CRO-09）红测：前端分诊卡 CoderTriageCard。
// 契约：plan §2 波 3 P2-d ①②③⑤＋设计块 2 §2.1/§3.4——
// 三按钮固定文案与顺序、第二按钮展开输入（空拦）、旧门退化两按钮（X2）、
// 「转入验证处理」收进「技术详情」二级折叠。
import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { CodingPendingGate } from "../state/coding-workspace-store";
import { CoderTriageCard } from "./CoderTriageCard";

const TRIAGE_SUMMARY_FIXTURE = {
  problem: "数据库连接被拒绝，迁移无法执行",
  findings: [
    { reason_code: "environment_port_blocked", message: "数据库连接被拒绝，迁移无法执行" },
  ],
  did: {
    changed_files: ["src/main.rs", "src/lib.rs", "src/api.rs", "tests/it.rs"],
    commit_sha: "a1b2c3d4e5f60718",
    verification_exit_code: null,
    environment_note: null,
  },
  unverified: ["cargo test 未运行", "迁移脚本未在真实库上验证"],
};

function operationalBlockerGate(overrides: Partial<CodingPendingGate> = {}): CodingPendingGate {
  return {
    gate_id: "coding_blocked_gate_0002",
    kind: "blocked",
    title: "运行环境阻塞",
    description: "Coder 报告运行环境阻塞（1 条 finding），流程已暂停: 数据库连接被拒绝",
    stage: "coding",
    role: "coder",
    expires_at: null,
    provider_snapshot: null,
    available_actions: [
      { action_id: "retry_coding", label: "重新启动 Coder", action_type: "retry_coding" },
      {
        action_id: "retry_coding_with_context",
        label: "我来说明情况，让 AI 继续",
        action_type: "retry_coding_with_context",
      },
      { action_id: "abort", label: "终止", action_type: "abort" },
    ],
    reason_code: "coding_operational_blocker",
    evidence_refs: [],
    raw_provider_output_ref: null,
    diagnostic: null,
    triage_summary: TRIAGE_SUMMARY_FIXTURE,
    ...overrides,
  };
}

function humanTriageGate(): CodingPendingGate {
  const gate = operationalBlockerGate();
  return {
    ...gate,
    title: "Coder 输出需要人工分诊",
    description: "Coder 输出需要人工分诊，流程已暂停",
    reason_code: "coding_output_human_triage",
    available_actions: [
      { action_id: "retry_coding", label: "重试 Coder", action_type: "retry_coding" },
      { action_id: "abort", label: "终止", action_type: "abort" },
    ],
  };
}

describe("CoderTriageCard（P2-d 分诊卡）", () => {
  it("coding_operational_blocker 渲染三按钮（固定文案与顺序）＋三段大白话；retry/abort 不带上下文", () => {
    const onRespond = vi.fn();
    render(<CoderTriageCard gate={operationalBlockerGate()} onRespond={onRespond} />);

    expect(screen.getByTestId("coder-triage-card")).toBeInTheDocument();
    const actions = screen.getByTestId("coder-triage-actions");
    const labels = within(actions)
      .getAllByRole("button")
      .map((button) => button.textContent);
    expect(labels).toEqual(["让 AI 再试一次", "我来说明情况，让 AI 继续", "终止"]);

    expect(screen.getByTestId("coder-triage-problem").textContent).toContain(
      "数据库连接被拒绝，迁移无法执行",
    );
    expect(screen.getByTestId("coder-triage-did").textContent).toContain("改了 4 个文件");
    expect(screen.getByTestId("coder-triage-unverified").textContent).toContain(
      "cargo test 未运行",
    );

    fireEvent.click(within(actions).getByRole("button", { name: "让 AI 再试一次" }));
    expect(onRespond).toHaveBeenCalledWith(
      "coding_blocked_gate_0002",
      "retry_coding",
      undefined,
    );
    fireEvent.click(within(actions).getByRole("button", { name: "终止" }));
    expect(onRespond).toHaveBeenCalledWith("coding_blocked_gate_0002", "abort", undefined);
    expect(onRespond).toHaveBeenCalledTimes(2);
  });

  it("第二按钮点击展开输入框；空输入提交被拦并提示「请先写一两句话」；填入后提交携带说明", () => {
    const onRespond = vi.fn();
    render(<CoderTriageCard gate={operationalBlockerGate()} onRespond={onRespond} />);

    expect(screen.queryByLabelText("补充说明")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "我来说明情况，让 AI 继续" }));
    const textarea = screen.getByLabelText("补充说明");
    expect(textarea).toBeInTheDocument();

    // 空输入：提交被拦并提示。
    fireEvent.click(screen.getByTestId("coder-triage-context-submit"));
    expect(onRespond).not.toHaveBeenCalled();
    expect(screen.getByTestId("coder-triage-context-hint").textContent).toContain(
      "请先写一两句话",
    );

    // 填入说明后提交：action_id 与去空白文本一并送达。
    fireEvent.change(textarea, { target: { value: "  数据库端口被占，请改用 5433  " } });
    fireEvent.click(screen.getByTestId("coder-triage-context-submit"));
    expect(onRespond).toHaveBeenCalledWith(
      "coding_blocked_gate_0002",
      "retry_coding_with_context",
      "数据库端口被占，请改用 5433",
    );
  });

  it("旧门仅 retry_coding/abort：退化为两按钮，第二按钮不渲染（X2 兼容在途 attempt）", () => {
    const onRespond = vi.fn();
    render(<CoderTriageCard gate={humanTriageGate()} onRespond={onRespond} />);

    const actions = screen.getByTestId("coder-triage-actions");
    const labels = within(actions)
      .getAllByRole("button")
      .map((button) => button.textContent);
    expect(labels).toEqual(["让 AI 再试一次", "终止"]);
    expect(
      screen.queryByRole("button", { name: "我来说明情况，让 AI 继续" }),
    ).toBeNull();
  });

  it("coding_output_human_triage 同卡渲染（无 summary 回落 description）", () => {
    const gate = { ...humanTriageGate(), triage_summary: null };
    render(<CoderTriageCard gate={gate} onRespond={vi.fn()} />);

    expect(screen.getByTestId("coder-triage-card")).toBeInTheDocument();
    expect(screen.getByTestId("coder-triage-problem").textContent).toContain(
      "Coder 输出需要人工分诊，流程已暂停",
    );
    expect(screen.queryByTestId("coder-triage-did")).toBeNull();
    expect(screen.queryByTestId("coder-triage-unverified")).toBeNull();
  });

  it("「转入验证处理」默认折叠不可见；展开技术详情＋二级折叠后可见且回调仍传 gate 身份", () => {
    const onEnter = vi.fn();
    render(
      <CoderTriageCard gate={humanTriageGate()} onRespond={vi.fn()} onEnterVerificationTriage={onEnter} />,
    );

    const technical = screen.getByTestId(
      "coder-triage-technical-details",
    ) as HTMLDetailsElement;
    expect(technical.open).toBe(false);
    const advanced = screen.getByTestId(
      "coder-triage-advanced-verification",
    ) as HTMLDetailsElement;
    expect(advanced.open).toBe(false);

    fireEvent.click(within(technical).getByText("技术详情"));
    expect(technical.open).toBe(true);
    fireEvent.click(within(advanced).getByText("高级：验证处理"));
    expect(advanced.open).toBe(true);

    fireEvent.click(screen.getByTestId("coding-verification-triage-enter"));
    expect(onEnter).toHaveBeenCalledWith("coding_blocked_gate_0002");
  });

  it("submitting 时三按钮禁用", () => {
    render(<CoderTriageCard gate={operationalBlockerGate({ submitting: true })} onRespond={vi.fn()} />);
    const actions = screen.getByTestId("coder-triage-actions");
    for (const button of within(actions).getAllByRole("button")) {
      expect(button).toBeDisabled();
    }
  });
});
