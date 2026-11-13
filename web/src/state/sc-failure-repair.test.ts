// P19b 前端 consumer RED 测试（P15→P18a/P19b 交接件；fixture 修正版）
//
// 范围（scope 固定）：仅新 wire 变体 sc_failure_repair_package 流经四个真实共享
// 入口，各自深等于同一份 ScFailureRepairPackage 正规化结果：
//   1) session_state → useWorkspaceStore.setSessionState（驾驶 store）
//   2) session_state → observerStateFromSessionState（观察 store；其消费面暴露
//      的是 workItemPlanArtifactVersions，不是 workItemPlanArtifact —— 后者
//      observerStateFromSessionState 从不设置，断言按真实入口属性取）
//   3) live artifact_update → handleWorkspaceWsMessage（WS 直播帧；当前生产
//      无该键臂时落 markdown 兜底分支，msg.markdown 为 undefined，
//      appendChatEntry 的 content_size 读取 undefined.length 抛 TypeError）
//   4) REST artifact → normalizeWorkItemPlanArtifactResponse
// 禁止且未使用：mock 回声（断言全部穿过真实 normalizer/store reducer）、
// 源码措辞 pin、纯 wire 类型拷贝、对 fixture 数据本身的测试。
//
// 红绿预期（controller 已实测的当前红）：
//   1) 驾驶路径：workItemPlanArtifact 得 null（normalizeWorkspaceArtifact 无臂）。
//   2) observer 路径：workItemPlanArtifactVersions 为空（同因）。
//   3) live 路径：handler 在 markdown 兜底分支抛 TypeError。
//   4) REST 路径：normalize 返回 null。
//   P19b 三处消费臂（helpers normalizeWorkspaceArtifact、
//   message-handler workItemPlanArtifactFromMessage、
//   ChatWorkspacePageParts normalizeWorkItemPlanArtifactResponse）落地后转绿。
//
// fixture 字节域：digest 常量为下方注明正文的真实 SHA-256（sha256sum 实算），
// bytes 为同一正文的真实 UTF-8 字节长度；completeness:"complete" 与
// missing_evidence:[] 相符；证据引用全为 status:"available"（无第三态）。

import { describe, expect, it, vi } from "vitest";
import { observerStateFromSessionState } from "./workspace-observer-store";
import { useWorkspaceStore } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";
import {
  handleWorkspaceWsMessage,
  type WsServerMessage,
} from "../hooks/workspace-ws-message-handler";
import { normalizeWorkItemPlanArtifactResponse } from "../pages/ChatWorkspacePageParts";
import type {
  ScFailureRepairPackage,
  WsOutMessage,
} from "../api/types";

// 真实 SHA-256（64 位小写 hex，sha256sum 实算）：
// raw 正文 = "provider raw output for session_sc_0001 round 3 (author delivery JSON body)"
const RAW_SHA256 =
  "8ac00815c423445bc4b591d05f0a3e90942865dbc16fad3c8b062395a215887a";
const RAW_BYTES = 75;
// source 正文 = "normalized compile delivery source for round 3 (prepare_author_delivery_for_compile)"
const SOURCE_SHA256 =
  "43682978bfbc7a90958c80f1cb277ae3baea42f064ebe8c57a2e0c51647033d6";
const SOURCE_BYTES = 84;
// evidence 日志字节 = rounds.jsonl 截至链终局条目的已验证前缀
const EVIDENCE_DIGEST =
  "9058f5b9512582a295bcef80e3c12678af9ceb9c08fcfaa241ac3396e6ac49f4";

const SC_FAILURE_REPAIR_PACKAGE: ScFailureRepairPackage = {
  schema_version: 1,
  package_id: "scfp_8ac00815",
  chain_id: "sc_chain_0001",
  session_id: "session_sc_0001",
  project_id: "project_0001",
  issue_id: "issue_0001",
  plan_id: "plan_0001",
  terminal_class: "compile",
  terminal_reason: "final compile failed: unresolved symbol sc_scope in plan payload",
  completeness: "complete",
  rounds: [
    {
      identity: {
        chain_id: "sc_chain_0001",
        round_id: "round_original_0001",
        node_id: "node_author_0001",
        round_ordinal: 1,
        provider: "claude_code",
        workspace_session_id: "session_sc_0001",
        role_run_seq: 3,
        run_ref: "work-item-split-runs/session_sc_0001/3",
      },
      launch_state: "started",
      failure_class: "compile",
      blocking_reasons: ["E001:12:unresolved symbol sc_scope"],
      repair_instructions: ["Bind sc_scope before re-running final compile"],
      raw: {
        status: "available",
        locator: "sc-split-output/v1/session_sc_0001/3",
        sha256: RAW_SHA256,
        bytes: RAW_BYTES,
      },
      source: {
        status: "available",
        locator: "sc-split-output/v1/session_sc_0001/3",
        sha256: SOURCE_SHA256,
        bytes: SOURCE_BYTES,
      },
      existing_failure_id: null,
      attempt_key: "attempt_0001",
      prompts: [],
      teaching_decision: "not_requested",
    },
  ],
  last_provider_output: {
    status: "available",
    locator: "sc-split-output/v1/session_sc_0001/3",
    sha256: RAW_SHA256,
    bytes: RAW_BYTES,
  },
  last_failure_source: {
    source_round_id: "round_original_0001",
    source: {
      status: "available",
      locator: "sc-split-output/v1/session_sc_0001/3",
      sha256: SOURCE_SHA256,
      bytes: SOURCE_BYTES,
    },
  },
  teaching_prompts: [],
  evidence_log_ref: "sc-failure-evidence/sc_chain_0001/rounds.jsonl",
  evidence_digest: EVIDENCE_DIGEST,
  evidence_through_entry_key: "chain-terminal",
  missing_evidence: [],
};

const NORMALIZED = {
  type: "sc_failure_repair_package" as const,
  payload: SC_FAILURE_REPAIR_PACKAGE,
};

type SessionStateFrameArtifact = Extract<
  WsOutMessage,
  { type: "session_state" }
>["artifact"];

// 同一份 frame 形状喂全部 session_state 入口（驾驶/观察共用，不许各自另造）；
// 字段集＝wire session_state 必填字段＋驾驶 store 最小模板（run_history 等，
// 镜像 workspace-ws-message-handler.test.ts 的 startLiveWorkItemPlanSession）。
function sessionStateFrame(artifact: SessionStateFrameArtifact) {
  return {
    type: "session_state" as const,
    session_id: "session_sc_0001",
    workspace_type: "work_item_plan",
    stage: "human_confirm",
    session_status: "waiting_for_human" as const,
    flow_kind: "single_candidate" as const,
    run_policy: "interactive" as const,
    run_history: {
      seen_fingerprints: [],
      repairs_used: 0,
      manual_repairs_used: 0,
      transitions_used: 0,
      initial_review_count: 0,
      verification_review_count: 0,
    },
    superpowers_enabled: false,
    openspec_enabled: false,
    messages: [],
    checkpoints: [],
    artifact,
    providers: { author: "claude_code" as const, reviewer: null },
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    active_run_id: null,
    human_presentation_revisions: [],
  };
}

const handlerOptions = () => ({
  invalidatedPreStageNodeIds: new Set<string>(),
  scheduleFlush: vi.fn(),
  streamFlushTimeouts: {},
});

describe("sc failure repair package consumer normalization (P19b)", () => {
  installWorkspaceStoreTestHooks();

  it("normalizes sc_failure_repair_package on the driving session_state path", () => {
    useWorkspaceStore.getState().setSessionState(
      sessionStateFrame({ sc_failure_repair_package: SC_FAILURE_REPAIR_PACKAGE }),
    );

    expect(useWorkspaceStore.getState().workItemPlanArtifact).toEqual(NORMALIZED);
  });

  it("normalizes the same payload on the observer session_state path", () => {
    const observed = observerStateFromSessionState(
      sessionStateFrame({ sc_failure_repair_package: SC_FAILURE_REPAIR_PACKAGE }),
    );

    // observer 的消费面入口是版本数组里的 current artifact（空版本表时由
    // workItemPlanVersionsFromSession 用 currentArtifact 合成 version 0）。
    const current = observed.workItemPlanArtifactVersions.find(
      (version) => version.is_current,
    );
    expect(current?.artifact).toEqual(NORMALIZED);
  });

  it("normalizes sc_failure_repair_package from a live artifact_update frame", () => {
    useWorkspaceStore.getState().setSessionState(sessionStateFrame(null));

    // 当前生产无该键臂：落 markdown 兜底分支读 undefined.length 抛
    // TypeError —— 真实行为红；P19b 臂落地后不再进入该分支。
    expect(() =>
      handleWorkspaceWsMessage(
        {
          type: "artifact_update",
          version: 4,
          sc_failure_repair_package: SC_FAILURE_REPAIR_PACKAGE,
        } as WsServerMessage,
        handlerOptions(),
      ),
    ).not.toThrow();

    expect(useWorkspaceStore.getState().workItemPlanArtifact).toEqual(NORMALIZED);
  });

  it("normalizes sc_failure_repair_package from the REST artifact response", () => {
    expect(
      normalizeWorkItemPlanArtifactResponse({
        sc_failure_repair_package: SC_FAILURE_REPAIR_PACKAGE,
      }),
    ).toEqual(NORMALIZED);
  });
});
