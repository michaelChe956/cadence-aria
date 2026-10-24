import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../../api/client";
import {
  ApiRequestError,
  findWorkspaceSessionIssueAddress,
  getAutomationEnrollmentControl,
  getCodingAttemptSnapshot,
  listVerificationTriageRecords,
  postAutomationContinuePlan,
  postCodingChoiceResponse,
  postCodingGateResponse,
  type AutomationControlStatus,
} from "../../api/client";
import type {
  CodingAttemptAddress,
  CodingAttemptSnapshotResponse,
} from "../../api/types";
import type * as CockpitShellModule from "../cockpit/CockpitShell";
import type { CockpitInboxItem } from "../../state/workspace-cockpit-projection";
import { codingAttentionItem } from "../../state/workspace-cockpit-projection";
import type { CodingAttentionItemDto } from "../../api/types/lifecycle";
import { WorkbenchInboxOverlay } from "./WorkbenchInboxOverlay";

// ── UI-3（待处理统一抽屉）：目录复用 Shell 投影、snapshot 是操作详情、
// choice 复用既有 REST、分诊卡 expected_version 取服务端 attempt_version ──

const directoryHolder: { items: CockpitInboxItem[] } = { items: [] };
const attentionOpenerHolder: { opener: ((item: unknown) => void) | null } = {
  opener: null,
};

vi.mock("../cockpit/CockpitShell", async (importOriginal) => {
  const actual = await importOriginal<typeof CockpitShellModule>();
  return {
    ...actual,
    useCockpitShellInbox: () => directoryHolder.items,
    useCockpitAttentionTargetOpener: () => attentionOpenerHolder.opener,
  };
});

vi.mock("../../api/client", async (importOriginal) => {
  const actual = await importOriginal<typeof ApiClient>();
  return {
    ...actual,
    getCodingAttemptSnapshot: vi.fn(),
    listVerificationTriageRecords: vi.fn(),
    postCodingGateResponse: vi.fn(),
    postCodingChoiceResponse: vi.fn(),
    getCodingChoiceResponseStatus: vi.fn(),
    findWorkspaceSessionIssueAddress: vi.fn(),
    getAutomationEnrollmentControl: vi.fn(),
    postAutomationContinuePlan: vi.fn(),
  };
});


const ADDRESS: CodingAttemptAddress = {
  projectId: "project_0002",
  issueId: "issue_0002",
  attemptId: "coding_attempt_0001",
};

function choiceAttentionDto(
  overrides: Partial<CodingAttentionItemDto> = {},
): CodingAttentionItemDto {
  return {
    key: "choice:attempt_a:choice_9",
    kind: "choice",
    project_id: ADDRESS.projectId,
    issue_id: ADDRESS.issueId,
    plan_id: null,
    attempt_id: ADDRESS.attemptId,
    logical_work_item_id: null,
    repository_name: "repo-b",
    reason_code: null,
    title: "选择请求待作答",
    summary: "是否补齐测试",
    occurred_at: "2026-10-24T00:00:00Z",
    target: { type: "coding_attempt", attempt_id: ADDRESS.attemptId },
    focus: { choice_id: "choice_9" },
    ...overrides,
  };
}

function gateAttentionDto(
  overrides: Partial<CodingAttentionItemDto> = {},
): CodingAttentionItemDto {
  return choiceAttentionDto({
    key: "gate:attempt_a:gate_7",
    kind: "triage",
    title: "AI 挂起了",
    summary: "Coder 长时间没有输出",
    reason_code: "coding_output_human_triage",
    focus: { gate_id: "gate_7" },
    ...overrides,
  });
}

function snapshotFixture(overrides: {
  pendingChoices?: CodingAttemptSnapshotResponse["pending_choices"];
  pendingGates?: CodingAttemptSnapshotResponse["pending_gates"];
}): CodingAttemptSnapshotResponse {
  return {
    pending_choices: overrides.pendingChoices ?? [],
    pending_gates: overrides.pendingGates ?? [],
  } as unknown as CodingAttemptSnapshotResponse;
}

function apiError(code: string, details: Record<string, unknown> = {}): ApiRequestError {
  return new ApiRequestError({ code, message: code, details });
}

const STOPPED_BINDING: NonNullable<AutomationControlStatus["binding"]> = {
  binding_version: 1,
  enrollment_id: "enrollment_0001",
  plan_id: "issue_work_item_plan_auto_1",
  session_id: "session_001",
  source: { stories: [], designs: [] },
  target: {
    kind: "logical_codebase",
    logical_codebase_id: "11111111-1111-1111-1111-111111111111",
    logical_repository_id: "00000000-0000-0000-0000-000000000001",
  },
  author_provider: "fake",
  reviewer_provider: "fake",
};

function stoppedContinueControl(
  overrides: Partial<AutomationControlStatus> = {},
): AutomationControlStatus {
  return {
    managed: true,
    binding: STOPPED_BINDING,
    plan_id: STOPPED_BINDING.plan_id,
    session_id: "session_001",
    terminated_at: null,
    stop_key: "plan_gate:session_001:human_confirm",
    stage: "stopped_needs_human",
    allowed_actions: ["continue_plan"],
    context_required: true,
    ...overrides,
  };
}

function renderOverlay(focus: Parameters<typeof WorkbenchInboxOverlay>[0]["focus"]) {
  return render(
    <WorkbenchInboxOverlay focus={focus} open onClose={() => undefined} />,
  );
}

describe("WorkbenchInboxOverlay（UI-3 待处理统一抽屉）", () => {
  beforeEach(() => {
    directoryHolder.items = [];
    attentionOpenerHolder.opener = null;
    vi.mocked(getCodingAttemptSnapshot).mockReset();
    vi.mocked(listVerificationTriageRecords).mockReset();
    vi.mocked(postCodingGateResponse).mockReset();
    vi.mocked(postCodingChoiceResponse).mockReset();
    vi.mocked(listVerificationTriageRecords).mockResolvedValue({
      attempt_id: ADDRESS.attemptId,
      attempt_version: 7,
      records: [],
    });
    vi.mocked(postCodingChoiceResponse).mockResolvedValue({
      command_id: "cmd-x",
      expected_run_id: "run-coding",
      choice_id: "choice_9",
      state: "delivered",
    });
    vi.mocked(findWorkspaceSessionIssueAddress).mockReset();
    vi.mocked(findWorkspaceSessionIssueAddress).mockResolvedValue({
      projectId: "project_0001",
      issueId: "issue_0001",
    });
    vi.mocked(getAutomationEnrollmentControl).mockReset();
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(
      stoppedContinueControl(),
    );
    vi.mocked(postAutomationContinuePlan).mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("目录复用 Shell 投影并按原分组渲染；focus 精确匹配行展开并拉取操作详情", async () => {
    const user = userEvent.setup();
    directoryHolder.items = [codingAttentionItem(choiceAttentionDto())];
    vi.mocked(getCodingAttemptSnapshot).mockResolvedValue(
      snapshotFixture({
        pendingChoices: [
          {
            gate_id: "coding_gate_1",
            choice_id: "choice_9",
            attempt_id: ADDRESS.attemptId,
            node_id: null,
            stage: "coding",
            role: "coder",
            provider: "fake",
            source: "ask_user_question",
            prompt: "编码拆分确认",
            options: [
              { id: "yes", label: "方案A" },
              { id: "no", label: "方案B" },
            ],
            allow_multiple: false,
            allow_free_text: false,
            status: "open",
            response: null,
            questions: [
              {
                id: "q-1",
                prompt: "是否补齐测试",
                options: [
                  { id: "yes", label: "方案A" },
                  { id: "no", label: "方案B" },
                ],
                allow_multiple: false,
                allow_free_text: false,
              },
            ],
            created_at: "2026-10-24T00:00:00Z",
            updated_at: "2026-10-24T00:00:00Z",
            expected_run_id: "run-coding",
          },
        ],
      }),
    );

    renderOverlay({ attemptId: ADDRESS.attemptId, choiceId: "choice_9" });

    // focus 自动展开 + 聚焦行高亮。
    const row = await screen.findByTestId("workbench-inbox-item-coding_action");
    expect(row).toHaveAttribute("data-focus-match", "true");
    expect(await screen.findByTestId("workbench-inbox-detail")).toBeInTheDocument();

    // 按完整 Attempt 地址拉 snapshot + 版本事实各一次。
    await waitFor(() => {
      expect(vi.mocked(getCodingAttemptSnapshot)).toHaveBeenCalledTimes(1);
    });
    expect(vi.mocked(getCodingAttemptSnapshot)).toHaveBeenCalledWith(ADDRESS);
    expect(vi.mocked(listVerificationTriageRecords)).toHaveBeenCalledWith(ADDRESS);

    // choice 复用既有 REST：expected_run_id 精确绑定 + answers 逐题结构。
    await user.click(await screen.findByRole("radio", { name: "方案A" }));
    await user.click(screen.getByRole("button", { name: "提交选择" }));

    await waitFor(() => {
      expect(postCodingChoiceResponse).toHaveBeenCalledTimes(1);
    });
    expect(postCodingChoiceResponse).toHaveBeenCalledWith(
      ADDRESS,
      "choice_9",
      {
        command_id: expect.any(String),
        expected_run_id: "run-coding",
        answers: [
          { question_id: "q-1", selected_option_ids: ["yes"], free_text: null },
        ],
      },
    );
  });

  it("同一 Attempt 多事项只拉一次 snapshot（复用一次，不按行重复拉取）", async () => {
    const user = userEvent.setup();
    directoryHolder.items = [
      codingAttentionItem(choiceAttentionDto()),
      codingAttentionItem(gateAttentionDto()),
    ];
    vi.mocked(getCodingAttemptSnapshot).mockResolvedValue(snapshotFixture({}));

    renderOverlay({ attemptId: ADDRESS.attemptId, choiceId: "choice_9" });
    await screen.findByTestId("workbench-inbox-detail");

    // 手动展开同 attempt 的第二行（分诊事项）。
    const rows = screen.getAllByTestId("workbench-inbox-item-coding_action");
    const gateRow = rows.find((node) => node.textContent?.includes("AI 挂起了"));
    expect(gateRow).toBeDefined();
    await user.click(
      within(gateRow as HTMLElement).getByTestId("workbench-inbox-item-toggle"),
    );

    // 两行详情都在（choice+gate），仍只拉取过一次 snapshot。
    expect((await screen.findAllByTestId("workbench-inbox-detail")).length).toBe(2);
    expect(vi.mocked(getCodingAttemptSnapshot)).toHaveBeenCalledTimes(1);
  });

  it("分诊门经 postCodingGateResponse 直接作答；expected_version 取服务端 attempt_version", async () => {
    const user = userEvent.setup();
    directoryHolder.items = [codingAttentionItem(gateAttentionDto())];
    vi.mocked(getCodingAttemptSnapshot).mockResolvedValue(
      snapshotFixture({
        pendingGates: [
          {
            gate_id: "gate_7",
            kind: "blocked",
            title: "Coder 输出待人工分诊",
            description: "输出不符合验收口径",
            stage: null,
            role: null,
            expires_at: null,
            provider_snapshot: null,
            available_actions: [
              { action_id: "act_retry", action_type: "retry_coding", label: "重试" },
              { action_id: "act_abort", action_type: "abort", label: "终止" },
            ],
            reason_code: "coding_output_human_triage",
            evidence_refs: [],
            raw_provider_output_ref: null,
            diagnostic: null,
            triage_summary: null,
          },
        ],
      }),
    );
    vi.mocked(postCodingGateResponse).mockResolvedValue({
      command_id: "cmd-x",
      state: "accepted",
      attempt_id: ADDRESS.attemptId,
      gate_id: "gate_7",
      action_id: "act_retry",
    });

    renderOverlay({ attemptId: ADDRESS.attemptId, gateId: "gate_7" });
    await user.click(await screen.findByRole("button", { name: "让 AI 再试一次" }));

    await waitFor(() => {
      expect(postCodingGateResponse).toHaveBeenCalledTimes(1);
    });
    expect(postCodingGateResponse).toHaveBeenCalledWith(
      ADDRESS,
      expect.objectContaining({
        gate_id: "gate_7",
        action_id: "act_retry",
        extra_context: null,
        // 7 来自 listVerificationTriageRecords.attempt_version，非 rework_count。
        expected_version: 7,
        command_id: "cmd-inbox-gate-coding_attempt_0001-gate_7-act_retry",
      }),
    );
  });

  it("版本事实缺失时分诊动作 fail-closed，不冒充 expected_version", async () => {
    const user = userEvent.setup();
    directoryHolder.items = [codingAttentionItem(gateAttentionDto())];
    vi.mocked(getCodingAttemptSnapshot).mockResolvedValue(
      snapshotFixture({
        pendingGates: [
          {
            gate_id: "gate_7",
            kind: "blocked",
            title: "Coder 输出待人工分诊",
            description: "输出不符合验收口径",
            stage: null,
            role: null,
            expires_at: null,
            provider_snapshot: null,
            available_actions: [
              { action_id: "act_retry", action_type: "retry_coding", label: "重试" },
            ],
            reason_code: "coding_output_human_triage",
            evidence_refs: [],
            raw_provider_output_ref: null,
            diagnostic: null,
            triage_summary: null,
          },
        ],
      }),
    );
    vi.mocked(listVerificationTriageRecords).mockRejectedValue(
      new Error("version endpoint down"),
    );

    renderOverlay({ attemptId: ADDRESS.attemptId, gateId: "gate_7" });
    await user.click(await screen.findByRole("button", { name: "让 AI 再试一次" }));

    await waitFor(() => {
      expect(screen.getByText(/expected_version 缺失/)).toBeInTheDocument();
    });
    expect(postCodingGateResponse).not.toHaveBeenCalled();
  });

  it("托管 abort 回 428 时打开整 plan 终止确认面（不渲染为错误）", async () => {
    const user = userEvent.setup();
    directoryHolder.items = [codingAttentionItem(gateAttentionDto())];
    vi.mocked(getCodingAttemptSnapshot).mockResolvedValue(
      snapshotFixture({
        pendingGates: [
          {
            gate_id: "gate_7",
            kind: "blocked",
            title: "Coder 输出待人工分诊",
            description: "输出不符合验收口径",
            stage: null,
            role: null,
            expires_at: null,
            provider_snapshot: null,
            available_actions: [
              { action_id: "act_abort", action_type: "abort", label: "终止" },
            ],
            reason_code: "coding_output_human_triage",
            evidence_refs: [],
            raw_provider_output_ref: null,
            diagnostic: null,
            triage_summary: null,
          },
        ],
      }),
    );
    vi.mocked(postCodingGateResponse).mockRejectedValue(
      apiError("automation_plan_termination_required", {
        enrollment_id: "enr_1",
        policy_revision: 3,
      }),
    );

    renderOverlay({ attemptId: ADDRESS.attemptId, gateId: "gate_7" });
    await user.click(await screen.findByRole("button", { name: "终止" }));

    expect(
      await screen.findByTestId("automation-termination-confirm"),
    ).toBeInTheDocument();
  });

  it("快照拉取失败就地报错、目录行保留并支持重试", async () => {
    const user = userEvent.setup();
    directoryHolder.items = [codingAttentionItem(choiceAttentionDto())];
    vi.mocked(getCodingAttemptSnapshot)
      .mockRejectedValueOnce(new Error("snapshot down"))
      .mockResolvedValueOnce(snapshotFixture({}));

    renderOverlay({ attemptId: ADDRESS.attemptId, choiceId: "choice_9" });

    const errorPanel = await screen.findByTestId("workbench-inbox-detail-error");
    expect(errorPanel).toHaveTextContent("操作详情拉取失败");
    // 目录行保留（拉取失败不删条目）。
    expect(screen.getByTestId("workbench-inbox-item-coding_action")).toBeInTheDocument();

    await user.click(within(errorPanel).getByRole("button", { name: "重试" }));
    expect(await screen.findByTestId("workbench-inbox-detail")).toBeInTheDocument();
  });

  it("收起驻留保留未提交表单：open=false 卸下抽屉壳但内容与选项保留", async () => {
    const user = userEvent.setup();
    directoryHolder.items = [codingAttentionItem(choiceAttentionDto())];
    vi.mocked(getCodingAttemptSnapshot).mockResolvedValue(
      snapshotFixture({
        pendingChoices: [
          {
            gate_id: "coding_gate_1",
            choice_id: "choice_9",
            attempt_id: ADDRESS.attemptId,
            node_id: null,
            stage: "coding",
            role: "coder",
            provider: "fake",
            source: "ask_user_question",
            prompt: "编码拆分确认",
            options: [
              { id: "yes", label: "方案A" },
              { id: "no", label: "方案B" },
            ],
            allow_multiple: false,
            allow_free_text: false,
            status: "open",
            response: null,
            questions: [],
            created_at: "2026-10-24T00:00:00Z",
            updated_at: "2026-10-24T00:00:00Z",
            expected_run_id: "run-coding",
          },
        ],
      }),
    );

    const view = render(
      <WorkbenchInboxOverlay
        focus={{ attemptId: ADDRESS.attemptId, choiceId: "choice_9" }}
        open
        onClose={() => undefined}
      />,
    );
    await user.click(await screen.findByRole("radio", { name: "方案A" }));
    expect(screen.getByRole("radio", { name: "方案A" })).toBeChecked();

    // 收起：抽屉壳退出 DOM（回归断言口径），内容驻留。
    view.rerender(
      <WorkbenchInboxOverlay
        focus={{ attemptId: ADDRESS.attemptId, choiceId: "choice_9" }}
        open={false}
        onClose={() => undefined}
      />,
    );
    expect(screen.queryByTestId("workbench-inbox-overlay")).toBeNull();
    expect(screen.getByTestId("workbench-inbox-overlay-parked")).toBeInTheDocument();

    // 重新展开：未提交的选择保留。
    view.rerender(
      <WorkbenchInboxOverlay
        focus={{ attemptId: ADDRESS.attemptId, choiceId: "choice_9" }}
        open
        onClose={() => undefined}
      />,
    );
    expect(screen.getByRole("radio", { name: "方案A" })).toBeChecked();
  });

  it("plan/workspace 目标不在抽屉内执行动作，提供统一导航下钻", async () => {
    const user = userEvent.setup();
    const openTarget = vi.fn();
    attentionOpenerHolder.opener = openTarget;
    directoryHolder.items = [
      codingAttentionItem(
        choiceAttentionDto({
          key: "plan-final:plan_9",
          kind: "plan_final_confirm",
          plan_id: "plan_9",
          attempt_id: null,
          target: { type: "coding_plan", plan_id: "plan_9" },
          focus: null,
          title: "等待人工最终确认",
        }),
      ),
    ];

    renderOverlay({ planId: "plan_9" });
    const button = await screen.findByTestId("workbench-inbox-open-target");
    await user.click(button);
    expect(openTarget).toHaveBeenCalledWith(
      expect.objectContaining({
        target: { type: "coding_plan", plan_id: "plan_9" },
      }),
    );
    expect(getCodingAttemptSnapshot).not.toHaveBeenCalled();
  });

  it("同一 choice 双来源只显示一次：有作答卡时同身份目录行丢弃", () => {
    const catalogRow = codingAttentionItem(choiceAttentionDto());
    const cardItem: CockpitInboxItem = {
      id: "session_001:choice:choice_9",
      kind: "choice",
      severity: 2,
      title: "选择请求待作答",
      summary: "是否补齐测试",
      triage: false,
      source: "choice",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: {
        sessionId: "session_001",
        choiceId: "choice_9",
        prompt: "是否补齐测试",
        expectedRunId: "run-coding",
        questions: [],
        allowMultiple: false,
        allowFreeText: false,
        source: "coding",
        attemptAddress: ADDRESS,
        status: "open",
      },
    };
    directoryHolder.items = [cardItem, catalogRow];

    renderOverlay({});
    expect(screen.getAllByTestId("workbench-inbox-item-choice")).toHaveLength(1);
    expect(
      screen.queryByTestId("workbench-inbox-item-coding_action"),
    ).toBeNull();
  });

  // 批 2 P5 补入口：托管 stopped 卡在共享抽屉内与 Cockpit 收件箱同源——
  // control 允许 continue_plan 即出「补说明并继续」，提交走同一 REST。
  it("托管 stopped 行提供补说明并继续（同一 control/continue-plan 通道）", async () => {
    const user = userEvent.setup();
    vi.mocked(postAutomationContinuePlan).mockResolvedValue({
      command_id: "cmd-inbox-continue-1",
      state: "accepted",
      session_id: "session_001",
    });
    directoryHolder.items = [
      {
        id: "session_001:stopped:session_001",
        kind: "stopped",
        severity: 2,
        title: "会话停在停点",
        summary: "停点原因：stopped_needs_human · 自动化托管中——在对话窗口补说明继续",
        triage: false,
        source: "session_status",
        createdAt: null,
        gate: null,
        inlineError: null,
        choice: null,
        automationManaged: true,
      },
    ];

    renderOverlay({});
    const row = screen.getByTestId("workbench-inbox-item-stopped");
    await user.click(await within(row).findByRole("button", { name: "补说明并继续" }));

    expect(getAutomationEnrollmentControl).toHaveBeenCalledWith(
      "project_0001",
      "issue_0001",
    );
    await user.type(
      await within(row).findByTestId("stopped-managed-continue-context"),
      "抽屉里补的说明",
    );
    await user.click(within(row).getByTestId("stopped-managed-continue-submit"));

    await waitFor(() => {
      expect(postAutomationContinuePlan).toHaveBeenCalledTimes(1);
    });
    expect(postAutomationContinuePlan).toHaveBeenCalledWith(
      "project_0001",
      "issue_0001",
      {
        command_id: expect.any(String),
        expected_binding: STOPPED_BINDING,
        expected_stop_key: "plan_gate:session_001:human_confirm",
        context: "抽屉里补的说明",
      },
    );
  });
});
