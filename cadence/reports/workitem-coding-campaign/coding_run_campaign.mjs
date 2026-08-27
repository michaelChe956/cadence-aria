#!/usr/bin/env node
/**
 * Coding campaign 单样本驱动器。
 * 此脚本只消费已确认的 Work Item Plan handoff，且不会为未知 gate 猜测恢复动作。
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HARD_LIMIT_MS = Number(
  process.env.ARIA_CODING_HARD_TIMEOUT_MS ?? process.env.ARIA_HARD_TIMEOUT_MS ?? 60 * 60_000,
);
const BASE = (process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317').replace(/\/$/, '');
const WS_BASE = (process.env.ARIA_WS_BASE_URL ?? BASE.replace(/^http/, 'ws')).replace(/\/$/, '');
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

async function runCampaign({ handoff, outRoot }) {
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
    if (Array.isArray(message.pending_gates) && message.pending_gates.length) {
      waitForUnknownGate(message.pending_gates, `${source}:pending_gates`);
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
        case 'coding_gate_required':
          waitForUnknownGate(message.gate ?? message, message.type);
          break;
        case 'coding_protocol_error':
          if (!automationStoppedForGate) fail('protocol_error', `${message.code ?? 'coding_protocol_error'}: ${message.message ?? ''}`);
          break;
        case 'plan_repair_required':
        case 'plan_amendment_updated':
          if (!automationStoppedForGate) fail('unhandled_coding_control_message', `Coding campaign 不定义自动策略: ${message.type}`);
          break;
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
      no_http_or_websocket_requests: true,
    }));
    return;
  }
  await runCampaign({ handoff, outRoot: options.outRoot });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}

export { outputTimestamp, preflightFailureOutDir };
