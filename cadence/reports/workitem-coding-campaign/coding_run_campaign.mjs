#!/usr/bin/env node
/**
 * Coding campaign 单样本驱动器。
 * 此脚本只消费已确认的 Work Item Plan handoff，且不会为未知 gate 猜测恢复动作。
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  campaignCommandId,
  parseHumanScript,
  stage3OutboundLogEntry,
} from './workitem_run_campaign.mjs';

const HARD_LIMIT_MS = Number(
  process.env.ARIA_CODING_HARD_TIMEOUT_MS ?? process.env.ARIA_HARD_TIMEOUT_MS ?? 60 * 60_000,
);
const BASE = (process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317').replace(/\/$/, '');
const WS_BASE = (process.env.ARIA_WS_BASE_URL ?? BASE.replace(/^http/, 'ws')).replace(/\/$/, '');
// durable 数据根（对齐 workitem_run_campaign.mjs 的 ARIA_ROOT=REPO_ROOT/.aria）：
// Finding 2 修复的 human-gate turn 记录回读对账从该根拼 .aria/projects/<p>/issues/<i>/
// workspace-sessions/<s>/human-gate-turns/*.json。
const CAMPAIGN_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(CAMPAIGN_DIR, '../../..');
const ARIA_ROOT = path.join(REPO_ROOT, '.aria');
const ACTIVE_STATUSES = new Set([
  'created',
  'running',
  'waiting_for_human',
  'blocked',
  'awaiting_manual_recovery',
  'awaiting_plan_amendment',
  'applying_plan_amendment',
  'amendment_apply_failed',
]);

function usageAndExit(message, code = 2) {
  console.error(`${message}\nUsage: node coding_run_campaign.mjs <handoff.json path> <outRoot> [--dry-run]`);
  process.exit(code);
}

function parseArgs(argv) {
  const args = argv.slice(2);
  const dryIndex = args.indexOf('--dry-run');
  const dryRun = dryIndex !== -1;
  if (dryRun) args.splice(dryIndex, 1);
  const unknown = args.find((arg) => arg.startsWith('--'));
  if (unknown) usageAndExit(`未知选项: ${unknown}`);
  if (args.length !== 2) usageAndExit('必须提供 handoff.json 路径与 outRoot。');
  if (!Number.isFinite(HARD_LIMIT_MS) || HARD_LIMIT_MS <= 0) {
    usageAndExit('ARIA_CODING_HARD_TIMEOUT_MS 必须是正整数毫秒数。');
  }
  const [handoffPath, outRoot] = args;
  if (!outRoot.trim()) usageAndExit('outRoot 不能为空。');
  return { handoffPath: path.resolve(handoffPath), outRoot: path.resolve(outRoot), dryRun };
}

function json(value) {
  return JSON.stringify(value, null, 2);
}

function now() {
  return new Date().toISOString();
}

function outputTimestamp() {
  return now().replace(/[-:.TZ]/g, '');
}

function preflightFailureOutDir(outRoot, provider, timestamp = outputTimestamp()) {
  return path.join(outRoot, `coding-${provider}-preflight-failed-${timestamp}`);
}

function errorText(error) {
  return error instanceof Error ? error.message : String(error);
}

function loadHandoff(handoffPath) {
  let handoff;
  try {
    handoff = JSON.parse(fs.readFileSync(handoffPath, 'utf8'));
  } catch (error) {
    throw new Error(`无法读取 handoff.json: ${errorText(error)}`);
  }
  const required = ['project_id', 'issue_id', 'plan_id', 'repository_id', 'provider'];
  const missing = required.filter((key) => typeof handoff[key] !== 'string' || !handoff[key]);
  if (missing.length) throw new Error(`handoff.json 缺少字段: ${missing.join(', ')}`);
  if (!Array.isArray(handoff.work_item_ids) || !handoff.work_item_ids.length) {
    throw new Error('handoff.json 必须含非空 work_item_ids。');
  }
  return handoff;
}

async function parseResponse(response, label) {
  const text = await response.text();
  let body = {};
  try {
    body = text ? JSON.parse(text) : {};
  } catch {
    body = { raw: text.slice(0, 1_000) };
  }
  if (!response.ok) {
    const detail = typeof body.message === 'string' ? body.message : text.slice(0, 500);
    throw new Error(`${label} HTTP ${response.status}${detail ? `: ${detail}` : ''}`);
  }
  return body;
}

async function requestJson(url, options, elapsedMs) {
  const remaining = HARD_LIMIT_MS - elapsedMs();
  if (remaining <= 0) throw new Error('hard-timeout before HTTP request');
  const response = await fetch(url, {
    ...options,
    signal: AbortSignal.timeout(Math.min(remaining, 60_000)),
  });
  return parseResponse(response, options.label ?? options.method ?? 'request');
}

function attemptIdOf(value) {
  const paths = ['attempt_id', 'attempt.attempt_id', 'id'];
  for (const dotted of paths) {
    const valueAtPath = dotted.split('.').reduce((current, key) => current?.[key], value);
    if (typeof valueAtPath === 'string' && valueAtPath) return valueAtPath;
  }
  return null;
}

function isConfirmed(status) {
  return typeof status === 'string' && status.toLowerCase() === 'confirmed';
}

async function verifyHandoffViaLifecycle(handoff, elapsedMs) {
  const lifecycle = await requestJson(
    `${BASE}/api/issues/${encodeURIComponent(handoff.issue_id)}/lifecycle?project_id=${encodeURIComponent(handoff.project_id)}`,
    { method: 'GET', label: 'read lifecycle before coding' },
    elapsedMs,
  );
  const plans = Array.isArray(lifecycle.work_item_plans) ? lifecycle.work_item_plans : [];
  const plan = plans.find((candidate) => (candidate.id ?? candidate.plan_id) === handoff.plan_id);
  if (!plan) throw new Error(`lifecycle 回读缺少 handoff plan: ${handoff.plan_id}`);
  if (!isConfirmed(plan.status)) throw new Error(`handoff plan 未 Confirmed: ${plan.status}`);
  const planWorkItemIds = Array.isArray(plan.work_item_ids) ? plan.work_item_ids : [];
  if (!planWorkItemIds.length) throw new Error('Confirmed plan 不包含 work items。');
  const lifecycleItems = new Set((lifecycle.work_items ?? []).map((item) => item.work_item_id ?? item.id));
  const missingFromPlan = handoff.work_item_ids.filter((id) => !planWorkItemIds.includes(id));
  const missingFromLifecycle = handoff.work_item_ids.filter((id) => !lifecycleItems.has(id));
  if (missingFromPlan.length || missingFromLifecycle.length) {
    throw new Error(`handoff work items 不存在或未归属 plan: plan缺少=${missingFromPlan.join(',') || '无'}; lifecycle缺少=${missingFromLifecycle.join(',') || '无'}`);
  }
  return { lifecycle, plan };
}

function resultTemplate(handoff, outDir) {
  return {
    project_id: handoff.project_id,
    issue_id: handoff.issue_id,
    plan_id: handoff.plan_id,
    repository_id: handoff.repository_id,
    provider: handoff.provider,
    attempt_id: null,
    outDir,
    startedAt: now(),
    finishedAt: null,
    elapsedSec: null,
    stageTimeline: [],
    gates: [],
    permissions: [],
    choices: [],
    review_results: [],
    worktree: { branch_name: null, base_branch: null, worktree_path: null, head_commit: null, push_status: null, review_request_url: null },
    usage: { usage_unavailable: true },
    timeline_nodes: [],
    failureClass: null,
    error: null,
    completed: false,
  };
}

function collectUsage(value, observations, source) {
  if (!value || typeof value !== 'object') return;
  if (Array.isArray(value)) {
    value.forEach((item) => collectUsage(item, observations, source));
    return;
  }
  if (value.kind === 'usage' && typeof value.output === 'string') {
    try { collectUsage(JSON.parse(value.output), observations, source); } catch { /* 按原始事件如实保留。 */ }
  }
  const input = Number.isInteger(value.input_tokens) ? value.input_tokens : value.prompt_tokens;
  const output = Number.isInteger(value.output_tokens) ? value.output_tokens : value.completion_tokens;
  const cache = Number.isInteger(value.cache_read_tokens) ? value.cache_read_tokens : value.cache_read_input_tokens;
  if ([input, output, cache].some((token) => Number.isInteger(token) && token >= 0)) {
    observations.push({
      source,
      input_tokens: Number.isInteger(input) && input >= 0 ? input : 0,
      output_tokens: Number.isInteger(output) && output >= 0 ? output : 0,
      cache_read_tokens: Number.isInteger(cache) && cache >= 0 ? cache : 0,
    });
  }
  Object.values(value).forEach((nested) => collectUsage(nested, observations, source));
}

function summarizeUsage(observations) {
  if (!observations.length) return { usage_unavailable: true };
  return {
    input_tokens: observations.reduce((total, item) => total + item.input_tokens, 0),
    output_tokens: observations.reduce((total, item) => total + item.output_tokens, 0),
    cache_read_tokens: observations.reduce((total, item) => total + item.cache_read_tokens, 0),
  };
}

function selectFirstChoice(message) {
  const options = Array.isArray(message.options) ? message.options : [];
  const selected = options[0];
  if (!selected?.id) return null;
  return { id: selected.id, label: selected.label ?? null };
}

function readinessIsComplete(message) {
  const readiness = message.group_final_readiness;
  return readiness?.status === 'complete' && Array.isArray(readiness.diagnostics) && readiness.diagnostics.length === 0;
}

// —— stage_gate（5s 自动放行门）识别 ——
// 现场证据（/tmp/aria-stage35-p1val/coding-pi-coding_attempt_716f.../ws.jsonl）：
// 服务器在 coding / code review 阶段前发 kind=stage_gate 的 coding_gate_required
// （gate_id 前缀 coding_stage_gate_），且 5s 窗口内的 coding_session_state.pending_gates
// 也携带同款门；服务器 5 秒后自动放行，流程照常推进到 final_confirm readiness complete。
// driver 把它当未知门停机（automationStoppedForGate）会哑到硬超时，因此识别为 stage_gate
// 的门只记审计不停机；其余未知门形态一律照旧 waitForUnknownGate 停机（fail-closed 不变）。
const STAGE_GATE_ID_PREFIX = 'coding_stage_gate_';

function isAutoReleasedStageGate(gate) {
  if (!gate || typeof gate !== 'object') return false;
  if (gate.kind === 'stage_gate') return true;
  return typeof gate.gate_id === 'string' && gate.gate_id.startsWith(STAGE_GATE_ID_PREFIX);
}

function pendingGatesPartition(pendingGates) {
  const gates = Array.isArray(pendingGates) ? pendingGates : [];
  const stageGates = [];
  const unknownGates = [];
  for (const gate of gates) {
    if (isAutoReleasedStageGate(gate)) stageGates.push(gate);
    else unknownGates.push(gate);
  }
  return { stageGates, unknownGates };
}

// —— Task 5.1 amendment 模式（ARIA_AMENDMENT_SCRIPT，8.4a 形态）——
// wire 依据：cadence/reports/workitem-conversational-gate-advance/evidence/amendment-wire-notes.md
// 三条 WS 线：① 原 plan session（session_link.parent_session_id）发 human_gate_feedback 过
// amendment turn；② child repair session（session_link.child_session_id）发
// confirm_plan_amendment 触发出版；③ coding WS 只收控制事件，resume 一律 durable 回读判定。

// 解析 ARIA_AMENDMENT_SCRIPT（语法复用 parseHumanScript）。未设置返回 null（零行为变化）；
// 设置后 fail-closed：仅接受 request-change:<文本> 与 confirm，且必须以 request-change 开始、
// 至少含一条 confirm（出版确认是 resume 的必要环节）。
function amendmentScriptFromEnv(env = process.env) {
  const raw = env.ARIA_AMENDMENT_SCRIPT;
  if (raw === undefined || raw === null) return null;
  if (String(raw).trim() === '') {
    throw new Error('ARIA_AMENDMENT_SCRIPT 不能为空（未设置即维持既有 fail-closed 行为；启用需提供 request-change:<文本> 与 confirm 动作）');
  }
  const actions = parseHumanScript(raw);
  for (const [index, action] of actions.entries()) {
    if (action.decision !== 'request-change' && action.decision !== 'confirm') {
      throw new Error(`ARIA_AMENDMENT_SCRIPT 第 ${index + 1} 条 ${action.decision} 在 amendment 链无对应动作（仅 request-change:<文本> 与 confirm）`);
    }
  }
  if (!actions.some((action) => action.decision === 'request-change')) {
    throw new Error('ARIA_AMENDMENT_SCRIPT 至少一条 request-change:<文本>（typed feedback 是修订候选的唯一来源）');
  }
  if (actions[0].decision !== 'request-change') {
    throw new Error('ARIA_AMENDMENT_SCRIPT 第 1 条必须以 request-change 开始（confirm 只能出现在修订轮次之后）');
  }
  if (!actions.some((action) => action.decision === 'confirm')) {
    throw new Error('ARIA_AMENDMENT_SCRIPT 至少一条 confirm（出版确认走 child repair session 的 confirm_plan_amendment）');
  }
  return actions;
}

// coding 控制消息的策略分派：未启用 amendment 模式时保持既有 fail-closed 零变化。
function codingControlMessagePlan({ amendmentActions }) {
  if (!amendmentActions) return { kind: 'fail', failureClass: 'unhandled_coding_control_message' };
  return { kind: 'amendment' };
}

// 线①：amendment command_id 由 (attemptId, actionIndex) 确定性生成；重连/重复发送/进程重启
// （同 attempt）都复用同值——服务端以 (session_id, command_id) durable 查重，重发回 Replayed+
// 同 turn turn_open，不启 provider、预算不重复扣。非空且远小于 256 bytes 上限。
function amendmentFeedbackCommandId({ attemptId, actionIndex }) {
  if (typeof attemptId !== 'string' || !attemptId.trim()) {
    throw new Error('amendmentFeedbackCommandId 要求非空 attemptId');
  }
  return campaignCommandId({
    campaignRunId: `coding-amendment:${attemptId}`,
    actionIndex,
    kind: 'human_gate_feedback',
  });
}

// 线① wire（逐字对齐 cheat-sheet ①）：{"type":"human_gate_feedback","command_id":C,"feedback":F}。
function amendmentGateFeedbackMessage({ commandId, feedback }) {
  if (typeof commandId !== 'string' || !commandId.trim()) {
    throw new Error('human_gate_feedback 必须携带非空 command_id（服务端校验非空且 ≤256 bytes）');
  }
  if (typeof feedback !== 'string' || !feedback) {
    throw new Error('human_gate_feedback 必须携带非空 feedback 文本');
  }
  return { type: 'human_gate_feedback', command_id: commandId, feedback };
}

// 线② wire（逐字对齐 cheat-sheet ②）：{"type":"confirm_plan_amendment","amendment_id":A}。
function amendmentConfirmMessage({ amendmentId }) {
  if (typeof amendmentId !== 'string' || !amendmentId.trim()) {
    throw new Error('confirm_plan_amendment 必须携带非空 amendment_id');
  }
  return { type: 'confirm_plan_amendment', amendment_id: amendmentId };
}

// workspace session WS 端点（原 plan session 与 child repair session 共用同一路由形态）。
function workspaceSessionWsUrl({ wsBase, sessionId }) {
  if (typeof sessionId !== 'string' || !sessionId.trim()) {
    throw new Error('workspaceSessionWsUrl 要求非空 sessionId');
  }
  return `${String(wsBase).replace(/\/$/, '')}/api/workspace-sessions/${encodeURIComponent(sessionId)}/ws`;
}

// 发现双源：plan_repair_required.session_link（完整对象，非 URL）与
// coding_session_state.linked_plan_repair（durable，重连可恢复）都能独立发现同一 repair。
function amendmentDiscoveryFromMessage(message) {
  if (!message || typeof message !== 'object') return null;
  if (message.type === 'plan_repair_required') {
    const link = message.session_link ?? null;
    const request = message.request ?? null;
    return {
      source: 'plan_repair_required',
      parent_session_id: link?.parent_session_id ?? null,
      child_session_id: link?.child_session_id ?? null,
      amendment_id: request?.amendment_id ?? null,
      repair_request_id: request?.id ?? null,
    };
  }
  if (message.type === 'coding_session_state') {
    const linked = message.linked_plan_repair ?? null;
    if (!linked) return null;
    return {
      source: 'coding_session_state',
      parent_session_id: linked.link?.parent_session_id ?? null,
      child_session_id: linked.link?.child_session_id ?? null,
      amendment_id: linked.amendment?.id ?? linked.request?.amendment_id ?? null,
      repair_request_id: linked.request?.id ?? null,
    };
  }
  return null;
}

function amendmentDiscoveryComplete(discovery) {
  return Boolean(discovery)
    && typeof discovery.parent_session_id === 'string' && discovery.parent_session_id.trim() !== ''
    && typeof discovery.child_session_id === 'string' && discovery.child_session_id.trim() !== '';
}

// 线① 发送计划：inFlight 未完结（含断线重连）→ 同 command 重发；否则游标落位的
// request-change → 新发送；其余（confirm/耗尽）→ 等待。
function amendmentGateFeedbackPlan({ actions, cursor, inFlight, turnStatus }) {
  if (inFlight) {
    return {
      kind: 'resend',
      actionIndex: inFlight.actionIndex,
      commandId: inFlight.commandId,
      reason: turnStatus === 'open' ? 'turn_open_reconnect' : 'unacked_reconnect',
    };
  }
  const action = actions[cursor];
  if (action?.decision === 'request-change') {
    return { kind: 'send', actionIndex: cursor, feedback: action.description };
  }
  return { kind: 'wait' };
}

// turn 完结推进游标；turn 失败仅当下一条仍是 request-change 才续发（不猜测恢复动作）。
function amendmentGateTurnOutcome({ actions, cursor, turnStatus }) {
  if (turnStatus === 'completed') return { kind: 'advanced', cursor: cursor + 1 };
  if (turnStatus === 'failed') {
    const next = actions[cursor + 1];
    if (next?.decision === 'request-change') return { kind: 'advanced', cursor: cursor + 1 };
    return { kind: 'fail', failureClass: 'amendment_turn_failed' };
  }
  throw new Error(`amendmentGateTurnOutcome 仅接受 completed/failed turn，实际为 ${String(turnStatus)}`);
}

// —— Finding 2 修复：durable human-gate turn 记录回读（模式对齐 workitem_run_campaign.mjs
// 的 stage3DurableReplayState）——服务端对 Replayed 一律回 human_gate_turn_open，不区分
// turn 是否已在断线窗口内完结（workspace_ws_handler/decisions.rs Replayed 分支）；driver
// 若只置 turnStatus='open' 等待 turn_completed 将永久停摆。缺目录/坏文件一律按无记录处理，
// 绝不抛出阻断恢复。
function amendmentDurableGateTurns({ ariaRoot, projectId, issueId, sessionId }) {
  if (![ariaRoot, projectId, issueId, sessionId].every((part) => typeof part === 'string' && part.trim())) {
    return [];
  }
  const directory = path.join(
    ariaRoot, 'projects', projectId, 'issues', issueId, 'workspace-sessions', sessionId, 'human-gate-turns',
  );
  const turns = [];
  let names;
  try {
    names = fs.readdirSync(directory);
  } catch {
    return []; // 目录不存在（尚无 turn 记录）是正常形态。
  }
  for (const name of names) {
    if (!name.endsWith('.json')) continue;
    try {
      const turn = JSON.parse(fs.readFileSync(path.join(directory, name), 'utf8'));
      if (turn?.command_id) {
        turns.push({
          command_id: turn.command_id,
          turn_id: turn.turn_id ?? null,
          status: turn.status ?? null,
          attempt_no: turn.attempt_no ?? null,
          budget_reserved: turn.budget_reserved ?? null,
          result_artifact_ref: turn.result_artifact_ref ?? null,
          failure_class: turn.failure_class ?? null,
        });
      }
    } catch {
      // 坏文件按无记录处理。
    }
  }
  turns.sort((left, right) => String(left.command_id).localeCompare(String(right.command_id)));
  return turns;
}

// 回读对账判定：按 command_id 匹配 inFlight。terminal（completed/failed）→ 直接用
// durable 结果推进；reserved/running → turn 仍活着，照常等 WS 事件；command 命中但
// turn_id 不一致 → conflict，durable 记录不可信，保守回退等待 WS 事件（不 fail-closed，
// 维持修复前行为）。turn 记录的 attempt_no/budget_reserved 随结果返回供审计。
function amendmentReplayedTurnReconciliation({ commandId, turnId, turns }) {
  if (typeof commandId !== 'string' || !commandId.trim()) return { kind: 'no_record' };
  const matched = (Array.isArray(turns) ? turns : []).filter((turn) => turn?.command_id === commandId);
  if (!matched.length) return { kind: 'no_record' };
  const mismatched = matched.find((turn) => turn.turn_id && turnId && turn.turn_id !== turnId);
  if (mismatched) {
    return { kind: 'conflict', observed_turn_id: turnId ?? null, record_turn_id: mismatched.turn_id ?? null };
  }
  const terminal = matched.find((turn) => turn.status === 'completed' || turn.status === 'failed');
  if (terminal) {
    return {
      kind: 'terminal',
      turn_id: terminal.turn_id ?? null,
      status: terminal.status,
      source: 'durable_turn_record',
      result_artifact_ref: terminal.result_artifact_ref ?? null,
      failure_class: terminal.failure_class ?? null,
    };
  }
  return { kind: 'open_live', turn_id: matched[0].turn_id ?? null };
}

// 线② 确认计划：仅 child stage=human_confirm 且游标落在 confirm 时发送；amendment id
// 双源（child 快照 amendment/request 与 coding 侧发现）均缺则 fail-closed；coding 侧已收到
// 该 amendment 的 plan_amendment_updated 时不再发送。
function amendmentChildConfirmPlan({ actions, cursor, childStage, amendmentId, codingEventAmendmentIds }) {
  if (childStage !== 'human_confirm') return { kind: 'wait', reason: 'child_stage_not_confirmable' };
  const action = actions[cursor];
  if (!action) return { kind: 'wait', reason: 'confirm_already_consumed' };
  if (action.decision !== 'confirm') return { kind: 'wait', reason: 'gate_turn_pending' };
  if (typeof amendmentId !== 'string' || !amendmentId.trim()) {
    return { kind: 'fail', failureClass: 'amendment_id_unresolved' };
  }
  if (codingEventAmendmentIds.has(amendmentId)) return { kind: 'wait', reason: 'coding_event_received' };
  return { kind: 'send', amendment_id: amendmentId, consume_cursor: cursor };
}

// 线② 重连重发：confirm 已发但 coding 侧未见该 amendment 的 durable 事件 → 重发同
// amendment_id（服务端幂等，且 Pending delivery 需新连接真实写成功才落 Delivered）。
function amendmentChildReconnectPlan({ childStage, amendmentId, codingEventAmendmentIds, confirmSentFor }) {
  if (childStage !== 'human_confirm') return { kind: 'wait', reason: 'child_stage_not_confirmable' };
  const target = amendmentId ?? confirmSentFor ?? null;
  if (!confirmSentFor || !target) return { kind: 'wait', reason: 'nothing_in_flight' };
  if (codingEventAmendmentIds.has(confirmSentFor)) return { kind: 'wait', reason: 'coding_event_received' };
  return { kind: 'resend', amendment_id: target };
}

// plan_amendment_updated manifest 提取（字段逐字对齐 wire；event_id 为 durable 固定值）。
function amendmentManifestFromUpdatedEvent(message) {
  if (typeof message?.event_id !== 'string' || !message.event_id.trim()) {
    throw new Error('plan_amendment_updated 缺少 event_id（durable event_id 形如 coding_plan_amendment_updated_{attempt}_{amendment}）');
  }
  const amendment = message.amendment ?? null;
  if (typeof amendment?.id !== 'string' || !amendment.id.trim()) {
    throw new Error('plan_amendment_updated 缺少 amendment.id');
  }
  return {
    id: amendment.id,
    repair_request_id: amendment.repair_request_id ?? null,
    previous_plan_revision_id: amendment.previous_plan_revision_id ?? null,
    new_plan_revision_id: amendment.new_plan_revision_id ?? null,
    resume_target: amendment.resume_target ?? null,
  };
}

// 按 event_id 幂等去重：断线重连后允许重收相同事件，driver 去重、不重发确认。
function amendmentNoteUpdatedEvent(seenEventIds, message) {
  const manifest = amendmentManifestFromUpdatedEvent(message);
  const eventId = message.event_id;
  const duplicate = seenEventIds.has(eventId);
  if (!duplicate) seenEventIds.add(eventId);
  return { duplicate, event_id: eventId, manifest };
}

// resume_target.mode → durable 判据（reexecute/revalidate/await_handoff，snake_case wire 值）。
// 未知/缺失模式不进入此表，由判定函数 fail-closed。
const AMENDMENT_RESUME_MODE_EXPECTATIONS = {
  reexecute: { attempt_status: 'running', stage: 'coding', unit_status: 'running' },
  revalidate: { stage: 'code_review', unit_status: 'needs_revalidation' },
  await_handoff: { unit_status: 'awaiting_amendment' },
};

// durable resume 判定：绝不凭 plan_amendment_updated 单一事件判成功；必须等
// coding_session_state/REST snapshot 满足同 attempt + status/stage/unit（按 resume_target.mode）
// 条件；快照内联 linked_plan_repair 时校验 binding 已指向新 revision。任何身份漂移 fail-closed。
function amendmentResumeJudgment({ expectedAttemptId, manifest, snapshot }) {
  if (!manifest || typeof manifest.id !== 'string' || !manifest.id) {
    return { kind: 'pending', reason: 'amendment_manifest_missing' };
  }
  const resumeTarget = manifest.resume_target ?? null;
  const mode = resumeTarget?.mode ?? null;
  const expectations = AMENDMENT_RESUME_MODE_EXPECTATIONS[mode];
  if (!expectations) {
    return { kind: 'fail', failureClass: 'amendment_resume_target_mode_unknown', mode };
  }
  if (!snapshot || typeof snapshot !== 'object') {
    return { kind: 'pending', reason: 'snapshot_missing' };
  }
  if (snapshot.attempt_id !== expectedAttemptId) {
    return { kind: 'fail', failureClass: 'amendment_resume_attempt_changed', snapshot_attempt_id: snapshot.attempt_id ?? null };
  }
  const targetWorkItemId = resumeTarget.logical_work_item_id;
  const units = Array.isArray(snapshot.units) ? snapshot.units : [];
  const resumeUnit = units.find((unit) => unit?.logical_work_item_id === targetWorkItemId);
  if (!resumeUnit) {
    return { kind: 'pending', reason: 'resume_target_unit_missing' };
  }
  const linkedNewRevision = snapshot.linked_plan_repair?.amendment?.new_plan_revision_id ?? null;
  if (linkedNewRevision && manifest.new_plan_revision_id && linkedNewRevision !== manifest.new_plan_revision_id) {
    return { kind: 'fail', failureClass: 'amendment_resume_binding_diverged', snapshot_new_plan_revision_id: linkedNewRevision };
  }
  const mismatches = [];
  if (resumeUnit.status !== expectations.unit_status) {
    mismatches.push(`unit:${resumeUnit.status}!=${expectations.unit_status}`);
  }
  if (expectations.stage && snapshot.stage !== expectations.stage) {
    mismatches.push(`stage:${snapshot.stage}!=${expectations.stage}`);
  }
  if (expectations.attempt_status && snapshot.status !== expectations.attempt_status) {
    mismatches.push(`status:${snapshot.status}!=${expectations.attempt_status}`);
  }
  if (mismatches.length) {
    return { kind: 'pending', reason: 'conditions_not_met', mismatches };
  }
  return {
    kind: 'resumed',
    evidence: {
      attempt_id: snapshot.attempt_id,
      status: snapshot.status,
      stage: snapshot.stage,
      unit_status: resumeUnit.status,
      amendment_id: manifest.id,
      new_plan_revision_id: manifest.new_plan_revision_id,
      resume_mode: mode,
    },
  };
}

// —— amendment 运行时：三条 WS 线的连接/重连/重发 wiring（决策全部委托上方纯函数）——
// hooks 由 runCampaign 提供：{ fail, log, noteUsage, elapsedSec }。
// projectId/issueId 供 Finding 2 的 durable turn 回读对账拼路径；deps 仅测试注入
// （WebSocketCtor/wsBase/ariaRoot），生产路径用全局 WebSocket/WS_BASE/ARIA_ROOT。
function createAmendmentRuntime({ attemptId, actions, hooks, projectId = null, issueId = null, deps = {} }) {
  const {
    WebSocketCtor = WebSocket,
    wsBase = WS_BASE,
    ariaRoot = ARIA_ROOT,
  } = deps;
  const RECONNECT_LIMIT = 40;
  const RECONNECT_DELAY_MS = 1_000;
  let cursor = 0;
  let inFlight = null;
  let turnStatus = null;
  let discovery = null;
  const discoverySources = new Set();
  let linesStarted = false;
  let closed = false;
  let parentSocket = null;
  let childSocket = null;
  // child socket 只在 onopen 后才可发送（真实 WebSocket 在 CONNECTING 态 send 会抛
  // InvalidStateError）；游标推进驱动的 confirm 若落在连接窗口内，交给 onopen 补发。
  let childSocketReady = false;
  let parentReconnects = 0;
  let childReconnects = 0;
  let childStage = null;
  let childAmendmentId = null;
  let confirmSentFor = null;
  let confirmSentAtSec = null;
  const seenEventIds = new Set();
  const codingEventAmendmentIds = new Set();
  const gateTurnAudit = [];
  let manifest = null;
  let resumeEvidence = null;

  const mergeDiscovery = (found) => {
    if (!discovery) {
      discovery = { parent_session_id: null, child_session_id: null, amendment_id: null, repair_request_id: null };
    }
    for (const key of ['parent_session_id', 'child_session_id', 'amendment_id', 'repair_request_id']) {
      if (!discovery[key] && found[key]) discovery[key] = found[key];
    }
    if (found.source) discoverySources.add(found.source);
  };
  const resolveAmendmentId = () => childAmendmentId ?? discovery?.amendment_id ?? null;

  const sendOn = (socket, line, message) => {
    if (closed || !socket) return;
    const logged = line === 'amendment_parent' ? stage3OutboundLogEntry(message) : message;
    hooks.log({ direction: 'out', line, message: logged });
    socket.send(JSON.stringify(message));
  };

  // —— 线①：原 plan session WS（session_link.parent_session_id）——
  const driveGate = () => {
    if (closed || !parentSocket) return;
    const plan = amendmentGateFeedbackPlan({ actions, cursor, inFlight, turnStatus });
    if (plan.kind === 'send') {
      const commandId = amendmentFeedbackCommandId({ attemptId, actionIndex: plan.actionIndex });
      inFlight = { actionIndex: plan.actionIndex, commandId, turnId: null };
      gateTurnAudit.push({ actionIndex: plan.actionIndex, command_id: commandId, sentSec: hooks.elapsedSec(), status: 'sent' });
      sendOn(parentSocket, 'amendment_parent', amendmentGateFeedbackMessage({ commandId, feedback: plan.feedback }));
    } else if (plan.kind === 'resend') {
      // 同 command 重发：服务端 durable 查重回 Replayed+同 turn turn_open，不启 provider、预算不重复扣。
      sendOn(parentSocket, 'amendment_parent', amendmentGateFeedbackMessage({
        commandId: plan.commandId,
        feedback: actions[plan.actionIndex].description,
      }));
      hooks.log({ event: 'amendment_gate_feedback_resent', command_id: plan.commandId, reason: plan.reason });
    }
  };

  const applyTurnOutcome = (outcome, message) => {
    const previous = inFlight;
    const audit = gateTurnAudit.find((entry) => entry.actionIndex === previous.actionIndex && entry.status === 'sent');
    if (audit) {
      audit.status = turnStatus;
      audit.turn_id = previous.turnId;
      audit.finishedSec = hooks.elapsedSec();
    }
    inFlight = null;
    turnStatus = null;
    if (outcome.kind === 'fail') {
      hooks.fail(outcome.failureClass, `amendment turn 失败且脚本无下一轮 request-change: ${json({
        actionIndex: previous.actionIndex,
        failure_class: message.failure_class ?? null,
        message: message.message ?? null,
      })}`);
      return;
    }
    cursor = outcome.cursor;
    hooks.log({ event: 'amendment_turn_finished', status: message.type, turn_id: previous.turnId, next_cursor: cursor });
    driveGate();
    // Finding 1 修复：游标推进可能把 confirm 动作落位，而 child 线的 human_confirm 快照
    // 可能早已到达（两条 socket 间无顺序保证）→ 必须在此用缓存状态重求值 child 确认
    // 计划并补发，否则 confirm 永不发送，只能烧到硬超时。
    driveChildConfirm();
  };

  // Finding 2 修复：父线重连/首发收到 turn_open 后，对 inFlight 做 durable turn 记录
  // 回读对账——服务端对 Replayed 一律回 turn_open 不区分 turn 是否已 Completed；
  // 若回读发现该 command 的 turn 已 terminal，直接用其结果推进，不等不会再来的 WS 事件。
  // 进程重启后首发同亦然：command_id 确定性复用命中 durable 查重，逐轮对账自愈。
  const reconcileGateTurnFromDurable = () => {
    if (closed || !inFlight) return;
    const turns = amendmentDurableGateTurns({
      ariaRoot,
      projectId,
      issueId,
      sessionId: discovery?.parent_session_id ?? null,
    });
    const verdict = amendmentReplayedTurnReconciliation({ commandId: inFlight.commandId, turnId: inFlight.turnId, turns });
    if (verdict.kind === 'terminal') {
      hooks.log({
        event: 'amendment_turn_reconciled_from_durable',
        command_id: inFlight.commandId,
        turn_id: verdict.turn_id,
        status: verdict.status,
        result_artifact_ref: verdict.result_artifact_ref,
        failure_class: verdict.failure_class,
      });
      turnStatus = verdict.status;
      applyTurnOutcome(
        amendmentGateTurnOutcome({ actions, cursor, turnStatus }),
        {
          type: verdict.status === 'completed' ? 'human_gate_turn_completed' : 'human_gate_turn_failed',
          turn_id: verdict.turn_id,
          failure_class: verdict.failure_class,
          message: 'durable human-gate turn 记录回读对账（断线窗口内已完结）',
        },
      );
      return;
    }
    if (verdict.kind === 'conflict') {
      hooks.log({
        event: 'amendment_turn_record_conflict',
        command_id: inFlight.commandId,
        observed_turn_id: verdict.observed_turn_id,
        record_turn_id: verdict.record_turn_id,
      });
    }
  };

  const handleParentInbound = (message) => {
    if (message.type === 'human_gate_turn_open') {
      if (inFlight && message.command_id === inFlight.commandId) {
        turnStatus = 'open';
        inFlight.turnId = typeof message.turn_id === 'string' ? message.turn_id : null;
        hooks.log({
          event: 'amendment_turn_open',
          command_id: message.command_id,
          turn_id: inFlight.turnId,
          remaining_budget: message.remaining_budget ?? null,
        });
        // Finding 2：turn_open 不区分 TurnOpened/Replayed、也不区分 turn 是否已在断线
        // 窗口内完结——回读 durable 记录对账，terminal 则直接推进。
        reconcileGateTurnFromDurable();
      }
      return;
    }
    if (message.type === 'human_gate_turn_completed' || message.type === 'human_gate_turn_failed') {
      if (inFlight && (inFlight.turnId === null || message.turn_id === inFlight.turnId)) {
        turnStatus = message.type === 'human_gate_turn_completed' ? 'completed' : 'failed';
        applyTurnOutcome(amendmentGateTurnOutcome({ actions, cursor, turnStatus }), message);
      }
      return;
    }
    if (message.type === 'human_gate_busy') {
      hooks.fail('amendment_gate_busy', `amendment 门 busy（存在未完结 turn）: ${json({ command_id: inFlight?.commandId ?? null })}`);
      return;
    }
    if (message.type === 'human_gate_closed') {
      hooks.fail('amendment_gate_closed', `amendment 门已关闭（coding application 完成后回 Confirmed）而脚本仍有未完成动作: ${json({ cursor })}`);
      return;
    }
    if (message.type === 'protocol_error') {
      hooks.fail('amendment_protocol_error', `${message.code ?? 'protocol_error'}: ${message.message ?? ''}`);
    }
  };

  const connectParent = () => {
    if (closed || !discovery?.parent_session_id) return;
    const socket = new WebSocketCtor(workspaceSessionWsUrl({ wsBase, sessionId: discovery.parent_session_id }));
    parentSocket = socket;
    socket.onopen = () => {
      if (closed || parentSocket !== socket) return;
      hooks.log({ event: 'amendment_parent_ws_open', session_id: discovery.parent_session_id });
      socket.send(JSON.stringify({ type: 'hello', session_id: discovery.parent_session_id, last_seen_node_id: null }));
      driveGate();
    };
    socket.onmessage = (event) => {
      if (closed || parentSocket !== socket) return;
      try {
        const message = JSON.parse(event.data);
        hooks.log({ direction: 'in', line: 'amendment_parent', message });
        hooks.noteUsage(message, 'amendment_parent');
        handleParentInbound(message);
      } catch (error) {
        hooks.fail('driver_error', error);
      }
    };
    socket.onclose = (event) => {
      if (closed || parentSocket !== socket) return;
      hooks.log({ event: 'amendment_parent_ws_close', code: event?.code, reason: event?.reason });
      parentSocket = null;
      if (inFlight) turnStatus = null;
      if (parentReconnects >= RECONNECT_LIMIT) {
        hooks.fail('amendment_parent_ws_reconnect_exhausted', `原 plan session WS 重连超过 ${RECONNECT_LIMIT} 次`);
        return;
      }
      parentReconnects += 1;
      setTimeout(() => connectParent(), RECONNECT_DELAY_MS);
    };
  };

  // —— 线②：child repair session WS（session_link.child_session_id）——
  // Finding 1 修复：child 确认计划的重驱动入口——child session_state 到达与游标推进
  // （applyTurnOutcome）两条路径都汇到此处，用缓存的 childStage/childAmendmentId 重
  // 求值；连接未就绪时不发送，由 onopen 补发。
  const driveChildConfirm = () => {
    if (closed || !childSocket || !childSocketReady) return;
    const plan = amendmentChildConfirmPlan({
      actions,
      cursor,
      childStage,
      amendmentId: resolveAmendmentId(),
      codingEventAmendmentIds,
    });
    if (plan.kind === 'send') {
      cursor = plan.consume_cursor + 1;
      confirmSentFor = plan.amendment_id;
      confirmSentAtSec = hooks.elapsedSec();
      sendOn(childSocket, 'amendment_child', amendmentConfirmMessage({ amendmentId: plan.amendment_id }));
    } else if (plan.kind === 'fail') {
      hooks.fail(plan.failureClass, `child 确认无法执行: ${json({ child_stage: childStage, amendment_id: resolveAmendmentId() })}`);
    }
  };

  const handleChildInbound = (message) => {
    if (message.type === 'session_state') {
      if (typeof message.stage === 'string') childStage = message.stage;
      const repair = message.plan_repair ?? null;
      const fromChild = repair?.amendment?.id ?? repair?.request?.amendment_id ?? null;
      if (fromChild) childAmendmentId = fromChild;
      driveChildConfirm();
      return;
    }
    if (message.type === 'protocol_error') {
      hooks.fail('amendment_confirm_rejected', `${message.code ?? 'protocol_error'}: ${message.message ?? ''}`);
    }
  };

  const connectChild = () => {
    if (closed || !discovery?.child_session_id) return;
    const socket = new WebSocketCtor(workspaceSessionWsUrl({ wsBase, sessionId: discovery.child_session_id }));
    childSocket = socket;
    socket.onopen = () => {
      if (closed || childSocket !== socket) return;
      childSocketReady = true;
      hooks.log({ event: 'amendment_child_ws_open', session_id: discovery.child_session_id });
      socket.send(JSON.stringify({ type: 'hello', session_id: discovery.child_session_id, last_seen_node_id: null }));
      // 断线重连重发：confirm 已发但 coding 侧未见 durable 事件 → 同 amendment_id 重发（投递重试）。
      const reconnectPlan = amendmentChildReconnectPlan({
        childStage,
        amendmentId: resolveAmendmentId(),
        codingEventAmendmentIds,
        confirmSentFor,
      });
      if (reconnectPlan.kind === 'resend') {
        sendOn(socket, 'amendment_child', amendmentConfirmMessage({ amendmentId: reconnectPlan.amendment_id }));
        hooks.log({ event: 'amendment_confirm_resent', amendment_id: reconnectPlan.amendment_id, reason: 'reconnect_delivery_retry' });
      }
      // Finding 1：游标已落在 confirm 而 confirm 尚未发出（如 child 快照先于 gate turn
      // 完结到达、或对账推进落在连接窗口内）→ 连接就绪即补发（此时 cursor 已消费则
      // 计划返回 confirm_already_consumed，不会与上面的重发叠加双发）。
      driveChildConfirm();
    };
    socket.onmessage = (event) => {
      if (closed || childSocket !== socket) return;
      try {
        const message = JSON.parse(event.data);
        hooks.log({ direction: 'in', line: 'amendment_child', message });
        hooks.noteUsage(message, 'amendment_child');
        handleChildInbound(message);
      } catch (error) {
        hooks.fail('driver_error', error);
      }
    };
    socket.onclose = (event) => {
      if (closed || childSocket !== socket) return;
      hooks.log({ event: 'amendment_child_ws_close', code: event?.code, reason: event?.reason });
      childSocket = null;
      childSocketReady = false;
      if (childReconnects >= RECONNECT_LIMIT) {
        hooks.fail('amendment_child_ws_reconnect_exhausted', `child repair session WS 重连超过 ${RECONNECT_LIMIT} 次`);
        return;
      }
      childReconnects += 1;
      setTimeout(() => connectChild(), RECONNECT_DELAY_MS);
    };
  };

  const startLines = () => {
    if (linesStarted || closed) return;
    linesStarted = true;
    connectParent();
    connectChild();
  };

  return {
    // 线③入口一：coding WS 控制事件（plan_repair_required / plan_amendment_updated）。
    onCodingControl(message) {
      if (closed) return;
      if (message.type === 'plan_repair_required') {
        const found = amendmentDiscoveryFromMessage(message);
        if (!found || !amendmentDiscoveryComplete(found)) {
          hooks.fail('plan_repair_required_link_missing', `plan_repair_required 缺少可用的 session_link（parent/child session id）: ${json({
            has_session_link: Boolean(message.session_link),
            parent_session_id: found?.parent_session_id ?? null,
            child_session_id: found?.child_session_id ?? null,
          })}`);
          return;
        }
        mergeDiscovery(found);
        hooks.log({
          event: 'amendment_discovered',
          source: found.source,
          parent_session_id: found.parent_session_id,
          child_session_id: found.child_session_id,
          amendment_id: found.amendment_id,
        });
        startLines();
        return;
      }
      if (message.type === 'plan_amendment_updated') {
        let noted;
        try {
          noted = amendmentNoteUpdatedEvent(seenEventIds, message);
        } catch (error) {
          hooks.fail('plan_amendment_updated_malformed', error);
          return;
        }
        if (noted.duplicate) {
          hooks.log({ event: 'amendment_event_deduped', event_id: noted.event_id });
          return;
        }
        manifest = noted.manifest;
        codingEventAmendmentIds.add(manifest.id);
        mergeDiscovery({
          source: 'plan_amendment_updated',
          parent_session_id: null,
          child_session_id: null,
          amendment_id: manifest.id,
          repair_request_id: manifest.repair_request_id,
        });
        hooks.log({
          event: 'amendment_manifest_recorded',
          event_id: noted.event_id,
          amendment_id: manifest.id,
          new_plan_revision_id: manifest.new_plan_revision_id,
          resume_target: manifest.resume_target,
        });
        // 不凭该事件判 resume：等待 coding_session_state / REST snapshot 的 durable 判据。
      }
    },
    // 线③入口二：durable 快照（coding WS coding_session_state 或 REST snapshot 归一化后）。
    onCodingState(snapshot) {
      if (closed) return;
      const linked = snapshot?.linked_plan_repair ?? null;
      if (linked) {
        const found = amendmentDiscoveryFromMessage({ type: 'coding_session_state', linked_plan_repair: linked });
        if (found) {
          const wasIncomplete = !amendmentDiscoveryComplete(discovery);
          mergeDiscovery(found);
          if (wasIncomplete && amendmentDiscoveryComplete(discovery)) {
            hooks.log({
              event: 'amendment_discovered',
              source: found.source,
              parent_session_id: discovery.parent_session_id,
              child_session_id: discovery.child_session_id,
              amendment_id: discovery.amendment_id,
            });
            startLines();
          }
        }
      }
      if (!manifest) return;
      const judgment = amendmentResumeJudgment({ expectedAttemptId: attemptId, manifest, snapshot });
      if (judgment.kind === 'resumed') {
        if (!resumeEvidence) {
          resumeEvidence = judgment.evidence;
          hooks.log({ event: 'amendment_resume_confirmed', ...judgment.evidence });
        }
        return;
      }
      if (judgment.kind === 'fail') {
        hooks.fail(judgment.failureClass, `amendment resume 判定失败关闭: ${json(judgment)}`);
      }
    },
    evidence() {
      return {
        mode: 'enabled',
        attempt_id: attemptId,
        script_actions: actions.length,
        discovered: discovery ? { ...discovery, sources: [...discoverySources] } : null,
        gate_turns: gateTurnAudit,
        confirm: confirmSentFor
          ? { amendment_id: confirmSentFor, sentSec: confirmSentAtSec, child_ws_reconnects: childReconnects }
          : null,
        amendment_event_ids: [...seenEventIds],
        manifest,
        resume: resumeEvidence,
      };
    },
    close() {
      closed = true;
      try { parentSocket?.close(); } catch { /* 已断开无需处理。 */ }
      try { childSocket?.close(); } catch { /* 已断开无需处理。 */ }
      parentSocket = null;
      childSocket = null;
    },
  };
}

function updateSnapshot(result, message) {
  if (message.branch_name !== undefined) result.worktree.branch_name = message.branch_name ?? null;
  if (message.base_branch !== undefined) result.worktree.base_branch = message.base_branch ?? null;
  if (message.worktree_path !== undefined) result.worktree.worktree_path = message.worktree_path ?? null;
  if (message.head_commit !== undefined) result.worktree.head_commit = message.head_commit ?? null;
  if (message.push_status !== undefined) result.worktree.push_status = message.push_status ?? null;
  if (message.review_request?.url !== undefined) result.worktree.review_request_url = message.review_request.url ?? null;
  if (Array.isArray(message.timeline_nodes)) result.timeline_nodes = message.timeline_nodes;
  if (Array.isArray(message.code_review_reports)) result.review_results.push(...message.code_review_reports);
  if (message.internal_pr_review) result.review_results.push(message.internal_pr_review);
}

async function runCampaign({ handoff, outRoot, amendmentActions = null }) {
  const started = Date.now();
  const elapsedMs = () => Date.now() - started;
  const elapsedSec = () => Number((elapsedMs() / 1_000).toFixed(3));
  let outDir = preflightFailureOutDir(outRoot, handoff.provider);
  let result = resultTemplate(handoff, outDir);
  let log = null;
  let ws = null;
  let hardTimer = null;
  let ended = false;
  let automationStoppedForGate = false;
  let initialStartSent = false;
  let reviewResumeSent = false;
  let finalConfirmSent = false;
  let currentStatus = null;
  const usage = [];

  const openOutput = (attemptId) => {
    outDir = path.join(outRoot, `coding-${handoff.provider}-${attemptId}`);
    result.outDir = outDir;
    result.attempt_id = attemptId;
    fs.mkdirSync(outDir, { recursive: true });
    log = fs.createWriteStream(path.join(outDir, 'ws.jsonl'), { flags: 'wx' });
  };
  const writeLog = (entry) => {
    if (log) log.write(`${JSON.stringify({ at: now(), ...entry })}\n`);
  };
  const writeOutputs = () => {
    fs.mkdirSync(outDir, { recursive: true });
    result.usage = summarizeUsage(usage);
    result.finishedAt = now();
    result.elapsedSec = elapsedSec();
    if (amendment) result.amendment = amendment.evidence();
    fs.writeFileSync(path.join(outDir, 'result.json'), json(result), 'utf8');
    const codingResult = {
      ...handoff,
      coding_attempt_id: result.attempt_id,
      coding_attempt_status: result.completed ? 'completed' : null,
      coding_outDir: outDir,
      coding_failure_class: result.failureClass,
      worktree: result.worktree,
    };
    fs.writeFileSync(path.join(outDir, 'coding-result.json'), json(codingResult), 'utf8');
  };
  const finish = (exitCode = 0) => {
    if (ended) return;
    ended = true;
    if (hardTimer) clearTimeout(hardTimer);
    if (!result.completed && !result.failureClass) result.failureClass = result.error ? 'driver_error' : 'incomplete';
    writeOutputs();
    amendment?.close();
    try { ws?.close(); } catch { /* 已断开无需处理。 */ }
    if (log) {
      log.end(() => process.exit(exitCode));
    } else {
      process.exit(exitCode);
    }
  };
  const fail = (failureClass, error, exitCode = 1) => {
    if (ended) return;
    if (!result.failureClass) result.failureClass = failureClass;
    if (!result.error) result.error = errorText(error);
    try {
      writeLog({ event: 'failure', failureClass: result.failureClass, error: result.error });
    } catch {
      // 日志流异常不能阻止 result.json 落盘。
    }
    finish(exitCode);
  };
  // amendment 模式（ARIA_AMENDMENT_SCRIPT）：三条 WS 线运行时；未启用时为 null，行为零变化。
  let amendment = null;
  const send = (message) => {
    if (ended || automationStoppedForGate) return;
    writeLog({ direction: 'out', message });
    ws.send(JSON.stringify(message));
  };
  const recordStage = (stage, source) => {
    if (typeof stage !== 'string') return;
    const previous = result.stageTimeline.at(-1);
    if (previous?.stage !== stage) {
      result.stageTimeline.push({ stage, elapsedSec: elapsedSec(), source });
      if (stage !== 'review_request') reviewResumeSent = false;
    }
  };
  const waitForUnknownGate = (gate, source) => {
    if (automationStoppedForGate) return;
    automationStoppedForGate = true;
    result.gates.push({ elapsedSec: elapsedSec(), source, gate, action: 'no_response_wait_for_hard_timeout' });
    writeLog({ event: 'automation_stopped_for_unknown_gate', source, gate });
  };
  // stage_gate 审计：同 gate_id 在 gate_required 与 5s 窗口内的 pending_gates 反复出现，
  // 按 gate_id 幂等去重，只在首次观测时落审计事件（缺 gate_id 时每次都记）。
  const observedStageGateIds = new Set();
  const observeStageGate = (gate, source) => {
    const gateId = typeof gate?.gate_id === 'string' && gate.gate_id ? gate.gate_id : null;
    if (gateId) {
      if (observedStageGateIds.has(gateId)) return;
      observedStageGateIds.add(gateId);
    }
    const title = typeof gate?.title === 'string' ? gate.title : null;
    result.gates.push({ elapsedSec: elapsedSec(), source, gate_id: gateId, title, kind: 'stage_gate', action: 'stage_gate_observed_wait_auto_release' });
    writeLog({ event: 'stage_gate_observed', source, gate_id: gateId, title, note: '服务器 5s 自动放行门，automation 不停机' });
  };
  const maybeDriveState = (message, source) => {
    const stage = message.stage;
    const status = message.status;
    if (typeof status === 'string') currentStatus = status;
    recordStage(stage, source);
    updateSnapshot(result, message);
    if (automationStoppedForGate) return;
    if (status === 'completed') {
      result.completed = true;
      result.failureClass = null;
      result.error = null;
      finish(0);
      return;
    }
    const { unknownGates, stageGates } = pendingGatesPartition(message.pending_gates);
    // 混排 [stage_gate, 未知门] 同批到达时，必须先记完 stage_gate 审计再对未知门停机，
    // 否则 unknownGates 先检即 return 会丢失 stage_gate_observed 审计事件。
    for (const gate of stageGates) observeStageGate(gate, `${source}:pending_gates`);
    if (unknownGates.length) {
      waitForUnknownGate(unknownGates, `${source}:pending_gates`);
      return;
    }
    if (stage === 'prepare_context' && !initialStartSent) {
      initialStartSent = true;
      send({ type: 'start_coding' });
      return;
    }
    if (stage === 'review_request' && ACTIVE_STATUSES.has(status) && !reviewResumeSent) {
      reviewResumeSent = true;
      send({ type: 'start_coding' });
      return;
    }
    if (stage === 'final_confirm' && status === 'waiting_for_human' && !finalConfirmSent) {
      if (!readinessIsComplete(message)) {
        fail('final_confirm_not_ready', 'final_confirm 未满足 group readiness complete 且 diagnostics 为空的确认条件');
        return;
      }
      finalConfirmSent = true;
      send({ type: 'final_confirm' });
      return;
    }
  };

  const refreshSnapshotAfterStageChange = async (attemptId) => {
    try {
      const snapshot = await requestJson(
        `${BASE}/api/projects/${encodeURIComponent(handoff.project_id)}/issues/${encodeURIComponent(handoff.issue_id)}/coding-attempts/${encodeURIComponent(attemptId)}`,
        { method: 'GET', label: 'refresh coding attempt after stage change' },
        elapsedMs,
      );
      const attempt = snapshot.attempt;
      if (!attempt) throw new Error('coding attempt snapshot 缺少 attempt');
      maybeDriveState({
        ...attempt,
        timeline_nodes: snapshot.timeline_nodes,
        code_review_reports: snapshot.code_review_reports,
        internal_pr_review: snapshot.internal_pr_review,
        review_request: snapshot.review_request,
        group_final_readiness: snapshot.group_final_readiness,
        pending_gates: snapshot.pending_gates,
      }, 'coding_stage_change_snapshot');
      // amendment 模式：REST snapshot 是 durable resume 判定的第二回读源（不含 linked_plan_repair）。
      if (amendment && !ended) {
        amendment.onCodingState({
          attempt_id: attempt.attempt_id ?? result.attempt_id,
          status: attempt.status,
          stage: attempt.stage,
          units: Array.isArray(snapshot.units) ? snapshot.units : [],
          linked_plan_repair: null,
        });
      }
    } catch (error) {
      fail('coding_snapshot_refresh_failed', error);
    }
  };

  hardTimer = setTimeout(() => {
    if (automationStoppedForGate) {
      fail('unknown_gate_timeout', `未知/不可理解 Coding gate 后等待至硬超时 ${HARD_LIMIT_MS}ms`);
    } else {
      fail('hard_timeout', `Coding 硬超时 ${HARD_LIMIT_MS}ms`);
    }
  }, HARD_LIMIT_MS);

  try {
    await verifyHandoffViaLifecycle(handoff, elapsedMs);
    const attempt = await requestJson(
      `${BASE}/api/projects/${encodeURIComponent(handoff.project_id)}/issues/${encodeURIComponent(handoff.issue_id)}/work-item-plans/${encodeURIComponent(handoff.plan_id)}/coding-attempts`,
      { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}', label: 'create group coding attempt' },
      elapsedMs,
    );
    const attemptId = attemptIdOf(attempt);
    if (!attemptId) throw new Error('create coding attempt 响应缺少 attempt_id');
    openOutput(attemptId);
    if (amendmentActions) {
      amendment = createAmendmentRuntime({
        attemptId,
        actions: amendmentActions,
        projectId: handoff.project_id,
        issueId: handoff.issue_id,
        hooks: {
          fail: (failureClass, error) => fail(failureClass, error),
          log: (entry) => writeLog(entry),
          noteUsage: (message, source) => collectUsage(message, usage, source),
          elapsedSec,
        },
      });
    }
    result.worktree = {
      branch_name: attempt.branch_name ?? null,
      base_branch: attempt.base_branch ?? null,
      worktree_path: attempt.worktree_path ?? null,
      head_commit: attempt.head_commit ?? null,
      push_status: attempt.push_status ?? null,
      review_request_url: attempt.review_request_url ?? null,
    };
    writeLog({ event: 'coding_attempt_created', attempt });

    if (handoff.execution_plan_confirm_required) {
      const executionPlan = await requestJson(
        `${BASE}/api/projects/${encodeURIComponent(handoff.project_id)}/issues/${encodeURIComponent(handoff.issue_id)}/coding-attempts/${encodeURIComponent(attemptId)}/execution-plan/confirm`,
        { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}', label: 'confirm execution plan' },
        elapsedMs,
      );
      writeLog({ event: 'execution_plan_confirmed', execution_plan: executionPlan });
    }

    if (elapsedMs() >= HARD_LIMIT_MS) throw new Error('hard-timeout before Coding WebSocket connection');
    ws = new WebSocket(
      `${WS_BASE}/ws/projects/${encodeURIComponent(handoff.project_id)}/issues/${encodeURIComponent(handoff.issue_id)}/coding-attempts/${encodeURIComponent(attemptId)}`,
    );
    ws.onopen = () => {
      writeLog({ event: 'ws_open' });
      send({ type: 'coding_hello', attempt_id: attemptId, last_seen_node_id: null });
    };
    ws.onmessage = (event) => {
      try {
        let message;
        try {
          message = JSON.parse(event.data);
        } catch {
          writeLog({ direction: 'in', malformed_raw: String(event.data) });
          fail('malformed_ws_message', `无法解析 Coding WebSocket 消息: ${String(event.data).slice(0, 500)}`);
          return;
        }
        writeLog({ direction: 'in', message });
        collectUsage(message, usage, message.type ?? 'unknown');
        switch (message.type) {
        case 'coding_session_state':
          maybeDriveState(message, message.type);
          // amendment 模式：发现双源之二（durable linked_plan_repair）+ 服务端 AmendmentApplyFailed
          // fail-closed + durable resume 判定（不凭单一事件判成功）。
          if (amendment && !ended) {
            if (message.status === 'amendment_apply_failed') {
              fail('amendment_apply_failed', '服务端 amendment application 失败（AmendmentApplyFailed）');
            } else {
              amendment.onCodingState({
                attempt_id: message.attempt_id ?? result.attempt_id,
                status: message.status,
                stage: message.stage,
                units: Array.isArray(message.units) ? message.units : [],
                linked_plan_repair: message.linked_plan_repair ?? null,
              });
            }
          }
          break;
        case 'coding_stage_change':
          recordStage(message.stage, message.type);
          if (!automationStoppedForGate && message.stage === 'review_request') {
            reviewResumeSent = false;
            if (ACTIVE_STATUSES.has(currentStatus)) {
              reviewResumeSent = true;
              send({ type: 'start_coding' });
            }
          }
          if (!automationStoppedForGate && message.stage === 'final_confirm') {
            // final_confirm 只在 API 回读同时满足 readiness/status 条件后才响应。
            void refreshSnapshotAfterStageChange(result.attempt_id);
          }
          break;
        case 'coding_timeline_node_created':
          if (message.node) result.timeline_nodes.push(message.node);
          break;
        case 'coding_timeline_node_updated':
        case 'coding_execution_event':
        case 'coding_stream_chunk':
        case 'coding_message_complete':
        case 'coding_choice_response_ack':
        case 'coding_chat_entry_created':
        case 'coding_provider_config_updated':
        case 'coding_pong':
          break;
        case 'coding_permission_request':
          if (!automationStoppedForGate) {
            result.permissions.push({ id: message.id, tool_name: message.tool_name, elapsedSec: elapsedSec(), approved: true });
            send({ type: 'permission_response', id: message.id, approved: true, reason: null });
          }
          break;
        case 'coding_choice_request': {
          if (automationStoppedForGate) break;
          const selected = selectFirstChoice(message);
          if (!selected) {
            fail('choice_without_options', `coding_choice_request ${message.id ?? '<missing>'} 没有可选项`);
            break;
          }
          result.choices.push({ id: message.id, elapsedSec: elapsedSec(), selected_option_ids: [selected.id], label: selected.label });
          send({ type: 'choice_response', id: message.id, selected_option_ids: [selected.id], free_text: null });
          break;
        }
        case 'code_review_complete':
          if (message.report) result.review_results.push(message.report);
          break;
        case 'review_request_update':
          if (message.review_request) result.review_results.push(message.review_request);
          if (message.review_request?.url) result.worktree.review_request_url = message.review_request.url;
          break;
        case 'internal_pr_review_complete':
          if (message.review) result.review_results.push(message.review);
          break;
        case 'coding_gate_required': {
          const gate = message.gate ?? message;
          if (isAutoReleasedStageGate(gate)) {
            observeStageGate(gate, message.type);
          } else {
            waitForUnknownGate(gate, message.type);
          }
          break;
        }
        case 'coding_protocol_error':
          if (!automationStoppedForGate) fail('protocol_error', `${message.code ?? 'coding_protocol_error'}: ${message.message ?? ''}`);
          break;
        case 'plan_repair_required':
        case 'plan_amendment_updated': {
          // 未设 ARIA_AMENDMENT_SCRIPT 时保持既有 fail-closed 零变化（回归锚点）。
          const controlPlan = codingControlMessagePlan({ amendmentActions, messageType: message.type });
          if (controlPlan.kind === 'fail') {
            if (!automationStoppedForGate) fail(controlPlan.failureClass, `Coding campaign 不定义自动策略: ${message.type}`);
            break;
          }
          amendment.onCodingControl(message);
          break;
        }
        default:
          if (!automationStoppedForGate) fail('unknown_ws_message', `未知 Coding WebSocket 消息类型: ${String(message.type)}`);
        }
      } catch (error) {
        fail('driver_error', error);
      }
    };
    ws.onerror = (event) => {
      if (!automationStoppedForGate) fail('ws_transport_error', event.message ?? 'Coding WebSocket error');
    };
    ws.onclose = (event) => {
      if (ended) return;
      writeLog({ event: 'ws_close', code: event?.code, reason: event?.reason, wasClean: event?.wasClean });
      if (!automationStoppedForGate) fail('ws_closed', `Coding WebSocket 关闭: code=${event?.code ?? 'unknown'}`);
    };
  } catch (error) {
    const message = errorText(error);
    fail(/timeout/i.test(message) ? 'hard_timeout' : 'setup_or_preflight_error', message);
  }
}

async function main() {
  const options = parseArgs(process.argv);
  let handoff;
  try {
    handoff = loadHandoff(options.handoffPath);
  } catch (error) {
    console.error(`启动校验失败: ${errorText(error)}`);
    process.exit(2);
  }
  // amendment 脚本启动即校验（含 --dry-run）；未设置时 amendmentActions 为 null，行为零变化。
  let amendmentActions = null;
  try {
    amendmentActions = amendmentScriptFromEnv();
  } catch (error) {
    console.error(`启动校验失败: ${errorText(error)}`);
    process.exit(2);
  }
  if (options.dryRun) {
    console.log(json({
      dry_run: true,
      handoff_path: options.handoffPath,
      project_id: handoff.project_id,
      issue_id: handoff.issue_id,
      plan_id: handoff.plan_id,
      work_item_ids: handoff.work_item_ids,
      provider: handoff.provider,
      execution_plan_confirm_required: Boolean(handoff.execution_plan_confirm_required),
      outDir_pattern: path.join(options.outRoot, `coding-${handoff.provider}-<attemptId>`),
      hard_timeout_ms: HARD_LIMIT_MS,
      amendment_script: {
        enabled: amendmentActions !== null,
        actions: amendmentActions ? amendmentActions.length : null,
      },
      no_http_or_websocket_requests: true,
    }));
    return;
  }
  await runCampaign({ handoff, outRoot: options.outRoot, amendmentActions });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}

export {
  amendmentChildConfirmPlan,
  amendmentChildReconnectPlan,
  amendmentConfirmMessage,
  amendmentDiscoveryComplete,
  amendmentDiscoveryFromMessage,
  amendmentDurableGateTurns,
  amendmentFeedbackCommandId,
  amendmentGateFeedbackMessage,
  amendmentGateFeedbackPlan,
  amendmentGateTurnOutcome,
  amendmentManifestFromUpdatedEvent,
  amendmentNoteUpdatedEvent,
  amendmentReplayedTurnReconciliation,
  amendmentResumeJudgment,
  amendmentScriptFromEnv,
  codingControlMessagePlan,
  createAmendmentRuntime,
  isAutoReleasedStageGate,
  outputTimestamp,
  pendingGatesPartition,
  preflightFailureOutDir,
  workspaceSessionWsUrl,
};
