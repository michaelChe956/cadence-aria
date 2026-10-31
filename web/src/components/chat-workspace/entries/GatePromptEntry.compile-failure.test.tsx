// owner 2026-10-31 真机反馈（0009 现场三处指引互相矛盾 + 「看不到报错内容」
// + 「确认点了必然再失败」死路）与 owner 两条意见：
// 1. 不应该出现这个成环的情况，应该拒绝——门卡必须一眼看出「候选无效」，
//    不是「待你确认」，且与普通 finding 分区呈现（不折叠、不进中性计数）。
// 2. 就算成环了要有醒目的提示，让用户重新生成，并提醒 design 可能需要重写、
//    重写时着重注意哪些（rewrite_guidance / owner_hint 与 author 教学重驱同源）。
//
// 本文件钉住 REQ-WSC-18 前端面：
// - metadata 有 `candidate_compile_failure` 即为「候选编译不过」态（平台侧
//   判据，前端不自猜）；
// - 顶部「现在请做 X」该态绝不说「确认当前版本可继续」，出路=让 author 按
//   要点重新生成 / 终止此门；
// - 原因行（gate-why）与状态行（gate-distance-approve）由同一份事实驱动，
//   不再与顶部指引各说各话；
// - 「确认当前版本」禁用 + 人话原因（fail-closed，点击不发 confirm），
//   反馈编辑器保留（重新生成的真实通路）。
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "../../../state/workspace-ws-store.test-utils";
import {
  GATE_CONFIRM_BLOCKED_COMPILE_FAILURE,
  GATE_NEXT_STEP_PREFIX,
} from "./gate-next-step";
import { GatePromptEntry } from "./GatePromptEntry";

/** owner 0009 现场（dependency_cycle）的最小事实投影。 */
const COMPILE_FAILURE_METADATA = {
  candidate_compile_failure: {
    code: "final_compile_prepare_failed",
    message:
      "WI-003 的 provider 指向了后仓的 WI-005，形成依赖环 WI-003→WI-005→WI-003；依赖必须按仓序从前往后声明",
    detail:
      "dependency_cycle: chain [WI-003, WI-005, WI-003] (failure_id=fc_0009)",
    rewrite_guidance: [
      "provider 只能指向前序仓的 Work Item",
      "重排 depends_on，使同仓顺序与仓序一致",
    ],
    owner_hint: "依赖环通常意味着 design 的依赖顺序也错了，design 可能需要一起重写",
  },
};

function gateEntry(metadata: Record<string, unknown> = {}): ChatEntry {
  return {
    id: "gate:entry",
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-10-31T00:00:00Z",
    metadata: {
      action_facade: "typed",
      command_id: "cmd_1",
      gate_trigger: "native_human_required",
      remaining_budget: 3,
      ...metadata,
    },
  };
}

function actions(): CockpitActionFacade {
  return {
    confirm: vi.fn(() => true),
    confirmReview: vi.fn(() => true),
    feedback: vi.fn(() => true),
    terminate: vi.fn(() => true),
    advance: vi.fn(() => true),
    adoptReview: vi.fn(() => true),
    confirmBatch: vi.fn(async () => undefined),
    recoverCompile: vi.fn(async () => undefined),
    recoverCandidate: vi.fn(),
    retryInitialization: vi.fn(async () => undefined),
    confirmTakeover: vi.fn(async () => undefined),
    rebind: vi.fn(),
    sendBootstrapAction: vi.fn(),
  };
}

/** 距通过清单渲染所需的 store 事实（plan 候选修订门）。 */
function seedPlanGateStore() {
  useWorkspaceStore.setState({
    workspaceType: "work_item_plan",
    humanGateSnapshot: {
      findings: [],
      repeated_fingerprints: [],
      attempts_used: 1,
      manual_repairs_remaining: 3,
      trigger: "native_human_required",
      resumable: true,
    },
  });
}

describe("候选编译不过：门卡醒目呈现「候选无效」（owner 2026-10-31 意见 1/2）", () => {
  installWorkspaceStoreTestHooks();

  it("独立红框给出「候选无效：需重新生成」+ 完整人话原因 + design 重写提醒 + 重写要点，均不在折叠区内", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry(COMPILE_FAILURE_METADATA)}
        actions={actions()}
      />,
    );

    // 醒目标题：候选无效、需重新生成——不是「待你确认」，也不藏在折叠区。
    const headline = screen.getByTestId("gate-compile-failure-headline");
    expect(headline).toHaveTextContent("候选无效");
    expect(headline).toHaveTextContent("需重新生成");
    expect(headline.closest("details")).toBeNull();

    // 机器码（可检索）与完整人话原因（不截断）直接可见。
    expect(screen.getByTestId("gate-compile-failure-code")).toHaveTextContent(
      "final_compile_prepare_failed",
    );
    const message = screen.getByTestId("gate-compile-failure-message");
    expect(message).toHaveTextContent(
      "WI-003 的 provider 指向了后仓的 WI-005，形成依赖环 WI-003→WI-005→WI-003",
    );
    expect(message.closest("details")).toBeNull();

    // owner 意见 2：design 可能需要一起重写的提醒 + 重新生成着重注意要点。
    expect(screen.getByTestId("gate-compile-failure-owner-hint")).toHaveTextContent(
      "design 可能需要一起重写",
    );
    const guidance = screen.getByTestId("gate-compile-failure-rewrite-guidance");
    expect(guidance).toHaveTextContent("重新生成时着重注意");
    expect(guidance).toHaveTextContent("provider 只能指向前序仓的 Work Item");
    expect(guidance).toHaveTextContent("重排 depends_on");

    // 完整诊断原文折叠可达、逐字不截断（owner 0009「看不到报错内容」修复）。
    expect(screen.getByTestId("gate-compile-failure-detail")).toHaveTextContent(
      "dependency_cycle: chain [WI-003, WI-005, WI-003] (failure_id=fc_0009)",
    );
  });

  it("畸形载荷（缺 code/message）不进该态——fail-closed 不猜", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry({
          candidate_compile_failure: { code: "final_compile_prepare_failed" },
        })}
        actions={actions()}
      />,
    );

    expect(screen.queryByTestId("gate-compile-failure")).toBeNull();
    // 回归护栏：普通门指引仍把确认当出路。
    expect(screen.getByTestId("gate-next-step")).toHaveTextContent(
      "确认当前版本即可继续",
    );
  });
});

describe("候选编译不过：三处指引同一份事实，不再互相矛盾", () => {
  installWorkspaceStoreTestHooks();

  it("顶部指引不说「可继续」，出路=让 author 按要点重新生成 / 终止此门", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry(COMPILE_FAILURE_METADATA)}
        actions={actions()}
      />,
    );

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent(GATE_NEXT_STEP_PREFIX);
    expect(guidance).toHaveTextContent("当前候选编译未通过");
    expect(guidance).toHaveTextContent("此候选无效");
    expect(guidance).toHaveTextContent("重新生成");
    // owner 0009 现场：顶部曾说「确认当前版本可继续」——此态绝不再出现。
    expect(guidance.textContent).not.toContain("可继续");
    expect(guidance.textContent).not.toContain("确认当前版本即可继续");
  });

  it("原因行说编译未通过、不能确认放行；不再出现「可直接确认」", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry(COMPILE_FAILURE_METADATA)}
        actions={actions()}
      />,
    );

    const why = screen.getByTestId("gate-why");
    expect(why).toHaveTextContent("Final Compile 未通过");
    expect(why).toHaveTextContent("此候选无效");
    expect(why).toHaveTextContent("不能确认放行");
    expect(why.textContent).not.toContain("可直接确认");
  });

  it("状态行（距通过 approve 项）不再说「可直接确认（或先反馈修订）」", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry(COMPILE_FAILURE_METADATA)}
        actions={actions()}
      />,
    );

    const approve = screen.getByTestId("gate-distance-approve");
    expect(approve).toHaveTextContent("候选编译未通过，不能确认放行");
    expect(approve.textContent).not.toContain("可直接确认");
  });

  it("「确认无需填写」的可选反馈提示在该态隐藏（确认已禁用，不得再暗示可确认）", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry(COMPILE_FAILURE_METADATA)}
        actions={actions()}
      />,
    );

    expect(screen.queryByTestId("gate-feedback-hint")).toBeNull();
  });

  it("编译不过优先于「反馈被拒」分支：指引不再把确认当被拒出路", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry({
          ...COMPILE_FAILURE_METADATA,
          rejectedFeedback: "把 WI-004 挪走",
          rejectionReason: "回复与 findings 无对应关系",
        })}
        actions={actions()}
      />,
    );

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("当前候选编译未通过");
    expect(guidance.textContent).not.toContain("没被采纳");
    expect(guidance.textContent).not.toContain("可继续");
    // 被拒事实仍如实呈现（红框区），只是不再当顶部指引。
    expect(screen.getByTestId("gate-rejection-reason")).toBeTruthy();
  });
});

describe("候选编译不过：「确认当前版本」禁用并给原因（平台侧判据，非死路）", () => {
  installWorkspaceStoreTestHooks();

  it("确认按钮禁用 + title/可见原因含出路；点击不发 confirm", () => {
    seedPlanGateStore();
    const gateActions = actions();
    render(
      <GatePromptEntry
        entry={gateEntry(COMPILE_FAILURE_METADATA)}
        actions={gateActions}
      />,
    );

    const confirm = screen.getByRole("button", { name: "确认当前版本" });
    expect(confirm).toBeDisabled();
    expect(confirm).toHaveAttribute("title", GATE_CONFIRM_BLOCKED_COMPILE_FAILURE);
    expect(screen.getByTestId("gate-confirm-blocked-reason")).toHaveTextContent(
      "暂不可确认：当前候选编译未通过",
    );
    expect(screen.getByTestId("gate-confirm-blocked-reason")).toHaveTextContent(
      "重新生成",
    );

    fireEvent.click(confirm);
    expect(gateActions.confirm).not.toHaveBeenCalled();
  });

  it("反馈编辑器与终止仍可用——重新生成/终止是真实出路，不是死路", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry(COMPILE_FAILURE_METADATA)}
        actions={actions()}
      />,
    );

    expect(screen.getByTestId("gate-feedback-editor")).toBeVisible();
    expect(screen.getByRole("button", { name: "终止此门" })).toBeVisible();
  });

  it("无 candidate_compile_failure 的普通门不受影响：确认可点、指引照旧", () => {
    seedPlanGateStore();
    const gateActions = actions();
    render(<GatePromptEntry entry={gateEntry()} actions={gateActions} />);

    const confirm = screen.getByRole("button", { name: "确认当前版本" });
    expect(confirm).toBeEnabled();
    expect(screen.queryByTestId("gate-confirm-blocked-reason")).toBeNull();
    expect(screen.getByTestId("gate-next-step")).toHaveTextContent(
      "确认当前版本即可继续",
    );
    fireEvent.click(confirm);
    expect(gateActions.confirm).toHaveBeenCalledOnce();
  });
});

describe("候选编译不过：自动重写两阶段提示（owner 2026-10-31 答复问题 1/2）", () => {
  installWorkspaceStoreTestHooks();

  it("阶段一（running）：首次失败即提示「正在自动重写/还剩 N 次/无需手动推」+ design 根因留意", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry({
          candidate_compile_failure: {
            ...COMPILE_FAILURE_METADATA.candidate_compile_failure,
            auto_rewrite: { phase: "running", remaining: 2, repeated: null },
          },
        })}
        actions={actions()}
      />,
    );

    // 提示的出现不依赖重写结果：第一次失败（重写进行中）即在场。
    expect(screen.getByTestId("gate-compile-failure")).toBeTruthy();
    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("平台正在自动重写");
    // 「还剩 N 次」数字来自后端事实（auto_rewrite.remaining），前端不自数。
    expect(guidance).toHaveTextContent("还剩 2 次");
    expect(guidance).toHaveTextContent("无需你手动推进");
    expect(guidance).toHaveTextContent("请留意是否需要修改 design");
    expect(guidance.textContent).not.toContain("可继续");

    // 红框阶段事实行：同样陈述剩余次数，醒目不折叠。
    const status = screen.getByTestId("gate-compile-failure-auto-rewrite");
    expect(status).toHaveTextContent("平台已启动自动重写，还剩 2 次");
    expect(status).toHaveTextContent("无需你手动推进");
    expect(status.closest("details")).toBeNull();

    // 禁用原因与阶段同源：running 不给手动出路（平台在自动重写）。
    expect(screen.getByTestId("gate-confirm-blocked-reason")).toHaveTextContent(
      "平台正在自动重写，无需你手动推进",
    );
    // 确认仍禁用（候选无效与重写进度无关）。
    expect(screen.getByRole("button", { name: "确认当前版本" })).toBeDisabled();
  });

  it("阶段二（stopped）：提示升级「自动重写无法解决（同一环已重复 N 次），请修改 design」，醒目不折叠", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry({
          candidate_compile_failure: {
            ...COMPILE_FAILURE_METADATA.candidate_compile_failure,
            auto_rewrite: { phase: "stopped", remaining: 0, repeated: 3 },
          },
        })}
        actions={actions()}
      />,
    );

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("自动重写无法解决同一依赖环");
    expect(guidance).toHaveTextContent("已重复 3 次");
    // 阶段二必须明确指向 owner 要动手（改 design），且确认不放行。
    expect(guidance).toHaveTextContent("请修改 design 的依赖顺序");
    expect(guidance.textContent).not.toContain("可继续");

    // 红框升级行：加粗红字陈述（次数同为后端事实）。
    const status = screen.getByTestId("gate-compile-failure-auto-rewrite");
    expect(status).toHaveTextContent("自动重写无法解决：同一依赖环已重复 3 次");
    expect(status).toHaveTextContent("请修改 design 后重新发起");
    expect(status.closest("details")).toBeNull();

    // 原因行同源升级（三处指引不矛盾）。
    expect(screen.getByTestId("gate-why")).toHaveTextContent(
      "自动重写无法解决同一依赖环，需修改 design 后重新生成",
    );
    expect(screen.getByTestId("gate-confirm-blocked-reason")).toHaveTextContent(
      "请修改 design 的依赖顺序后重新发起",
    );
    expect(screen.getByRole("button", { name: "确认当前版本" })).toBeDisabled();
  });

  it("auto_rewrite 相位值畸形时按无该信息处理（不猜 running/stopped，回退单阶段文案）", () => {
    seedPlanGateStore();
    render(
      <GatePromptEntry
        entry={gateEntry({
          candidate_compile_failure: {
            ...COMPILE_FAILURE_METADATA.candidate_compile_failure,
            auto_rewrite: { phase: "paused" as unknown as "running", remaining: 5 },
          },
        })}
        actions={actions()}
      />,
    );

    expect(screen.queryByTestId("gate-compile-failure-auto-rewrite")).toBeNull();
    expect(screen.getByTestId("gate-next-step")).toHaveTextContent(
      "让 author 按下方要点重新生成",
    );
    expect(screen.getByTestId("gate-next-step").textContent).not.toContain("可继续");
  });
});
