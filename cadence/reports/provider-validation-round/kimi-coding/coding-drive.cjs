// 2b 全功能持久 coding 驱动（campaign 驱动的接管变体）：
// 背景——v27 事件流绑定「触发连接」（spawn_coding_runner 绑该连接 event_tx；无广播）。
// 外部工具（recover/gate-release）触发 runner 后，campaign 驱动重挂载是盲的（只见首帧）。
// 因此本驱动在同一条持久连接上完成：blocked 门分诊（retry_coding）+ AMR 恢复
// （recover_coding）+ 权限批准 + 选项应答 + review_request 续跑 + final_confirm +
// usage_by_role 采集 + 终态观察。wire 形状逐字镜像 coding_run_campaign.mjs。
// 用法：node coding-drive.cjs <project_id> <issue_id> <attempt_id> <outDir>
'use strict';
const fs = require('node:fs');
const path = require('node:path');

const BASE = (process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317').replace(/\/$/, '');
const WS_BASE = (process.env.ARIA_WS_BASE_URL ?? BASE.replace(/^http/, 'ws')).replace(/\/$/, '');
const HARD_LIMIT_MS = Number(process.env.ARIA_CODING_DRIVE_HARD_TIMEOUT_MS ?? 110 * 60_000);
const MAX_GATE_RELEASES = Number(process.env.ARIA_CODING_DRIVE_MAX_GATE_RELEASES ?? 4);
const MAX_RECOVERS = Number(process.env.ARIA_CODING_DRIVE_MAX_RECOVERS ?? 6);
const [projectId, issueId, attemptId, outDir] = process.argv.slice(2);
if (![projectId, issueId, attemptId, outDir].every((v) => v)) {
  console.error('Usage: node coding-drive.cjs <project_id> <issue_id> <attempt_id> <outDir>');
  process.exit(2);
}
fs.mkdirSync(outDir, { recursive: true });
const stamp = new Date().toISOString().replace(/[-:.TZ]/g, '');
const log = fs.createWriteStream(path.join(outDir, `coding-drive-${stamp}.jsonl`), { flags: 'wx' });
const writeLog = (entry) => log.write(`${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`);
console.log('LOG=' + path.join(outDir, `coding-drive-${stamp}.jsonl`));

const TERMINAL = new Set(['completed', 'aborted', 'failed']);
const ACTIVE = new Set(['created', 'running', 'waiting_for_human', 'blocked', 'awaiting_manual_recovery', 'awaiting_plan_amendment', 'applying_plan_amendment', 'amendment_apply_failed']);
const state = {
  status: null,
  stage: null,
  wi: null,
  gateReleases: 0,
  recovers: 0,
  respondedGates: new Set(),
  finalConfirmSent: false,
  reviewResumeSent: false,
  usageByRole: {},
  reviewEvents: [],
  stageTimeline: [],
  permissionCount: 0,
  choiceCount: 0,
  unknownTypes: new Set(),
  started: Date.now(),
};
let ws = null;
let ended = false;
let reconnects = 0;

const hardTimer = setTimeout(() => finish(3, `coding-drive 硬超时 ${HARD_LIMIT_MS}ms`), HARD_LIMIT_MS);

function finish(code, reason) {
  if (ended) return;
  ended = true;
  clearTimeout(hardTimer);
  const summary = {
    reason,
    code,
    status: state.status,
    stage: state.stage,
    current_work_item_id: state.wi,
    gate_releases: state.gateReleases,
    recovers: state.recovers,
    permissions_approved: state.permissionCount,
    choices_answered: state.choiceCount,
    final_confirm_sent: state.finalConfirmSent,
    usage_by_role: Object.keys(state.usageByRole).length ? state.usage_by_role : { usage_unavailable: true },
    review_events: state.reviewEvents,
    stage_timeline: state.stageTimeline,
    unknown_message_types: [...state.unknownTypes],
  };
  writeLog({ event: 'finish', ...summary });
  fs.writeFileSync(path.join(outDir, `coding-drive-summary-${stamp}.json`), JSON.stringify(summary, null, 2));
  try { ws?.close(); } catch { /* noop */ }
  log.end(() => process.exit(code));
}

function send(message) {
  writeLog({ direction: 'out', message });
  ws.send(JSON.stringify(message));
}

function recordStage(stage) {
  if (!stage || state.stageTimeline.at(-1)?.stage === stage) return;
  state.stageTimeline.push({ stage, at: new Date().toISOString(), elapsedSec: Math.round((Date.now() - state.started) / 1000) });
  writeLog({ event: 'stage', stage });
}

function collectUsage(message) {
  const event = message.event;
  if (!event || event.kind !== 'usage' || typeof event.output !== 'string') return;
  try {
    const parsed = JSON.parse(event.output);
    if (parsed && typeof parsed.role === 'string') {
      state.usageByRole[parsed.role] = {
        input_tokens: parsed.input_tokens ?? null,
        output_tokens: parsed.output_tokens ?? null,
        cache_read_tokens: parsed.cache_read_tokens ?? null,
        cache_creation_tokens: parsed.cache_creation_tokens ?? null,
      };
      writeLog({ event: 'usage_collected', role: parsed.role });
    }
  } catch (error) {
    writeLog({ event: 'usage_parse_failed', error: String(error) });
  }
}

function driveSessionState(message) {
  state.status = message.status ?? state.status;
  state.stage = message.stage ?? state.stage;
  state.wi = message.current_work_item_id ?? state.wi;
  if (message.stage) recordStage(message.stage);

  // blocked 门分诊：pending_gates 含 kind=blocked → retry_coding（预算内，幂等去重）。
  if (state.status === 'blocked') {
    const gates = Array.isArray(message.pending_gates) ? message.pending_gates : [];
    const blocked = gates.find((g) => g?.kind === 'blocked' && !state.respondedGates.has(g.gate_id));
    if (blocked) {
      if (state.gateReleases >= MAX_GATE_RELEASES) {
        writeLog({ event: 'gate_release_budget_exhausted', gate_id: blocked.gate_id, releases: state.gateReleases });
        return;
      }
      state.respondedGates.add(blocked.gate_id);
      state.gateReleases += 1;
      writeLog({ event: 'gate_triage_release', gate_id: blocked.gate_id, action: 'retry_coding', release_no: state.gateReleases });
      send({ type: 'gate_response', gate_id: blocked.gate_id, action_id: 'retry_coding', extra_context: null });
      return;
    }
    return;
  }
  if (state.status === 'awaiting_manual_recovery') {
    if (state.recovers >= MAX_RECOVERS) {
      writeLog({ event: 'recover_budget_exhausted', recovers: state.recovers });
      return;
    }
    state.recovers += 1;
    writeLog({ event: 'recover_send', recover_no: state.recovers });
    send({ type: 'recover_coding' });
    return;
  }
  if (state.stage === 'review_request' && ACTIVE.has(state.status) && !state.reviewResumeSent) {
    state.reviewResumeSent = true;
    writeLog({ event: 'review_resume_send' });
    send({ type: 'start_coding' });
    return;
  }
  if (state.stage === 'final_confirm' && state.status === 'waiting_for_human' && !state.finalConfirmSent) {
    const readiness = message.group_final_readiness;
    const ready = readiness?.status === 'complete' && Array.isArray(readiness.diagnostics) && readiness.diagnostics.length === 0;
    if (ready) {
      state.finalConfirmSent = true;
      writeLog({ event: 'final_confirm_send' });
      send({ type: 'final_confirm' });
    } else {
      writeLog({ event: 'final_confirm_not_ready', readiness });
    }
  }
}

function connect() {
  const url = `${WS_BASE}/ws/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/coding-attempts/${encodeURIComponent(attemptId)}`;
  ws = new WebSocket(url);
  ws.onopen = () => {
    writeLog({ event: 'ws_open', url });
    ws.send(JSON.stringify({ type: 'coding_hello', attempt_id: attemptId, last_seen_node_id: null }));
  };
  ws.onmessage = (event) => {
    let message;
    try { message = JSON.parse(String(event.data)); } catch { return; }
    const type = message.type ?? 'unknown';
    switch (type) {
      case 'coding_session_state':
        writeLog({ event: 'session_state', status: message.status, stage: message.stage, wi: message.current_work_item_id });
        driveSessionState(message);
        if (TERMINAL.has(message.status ?? '')) {
          writeLog({ event: 'attempt_terminal', status: message.status });
          setTimeout(() => finish(message.status === 'completed' ? 0 : 1, `attempt terminal ${message.status}`), 3_000);
        }
        return;
      case 'coding_stage_change':
        recordStage(message.stage);
        if (message.stage === 'review_request') state.reviewResumeSent = false;
        return;
      case 'coding_permission_request':
        state.permissionCount += 1;
        send({ type: 'permission_response', id: message.id, approved: true, reason: null });
        return;
      case 'coding_choice_request': {
        const options = Array.isArray(message.options) ? message.options : [];
        const selected = options[0];
        if (selected?.id) {
          state.choiceCount += 1;
          send({ type: 'choice_response', id: message.id, selected_option_ids: [selected.id], free_text: null });
        } else {
          writeLog({ event: 'choice_without_options', id: message.id });
        }
        return;
      }
      case 'coding_gate_required': {
        const gate = message.gate ?? message;
        const isStageGate = gate?.kind === 'stage_gate' || String(gate?.gate_id ?? '').startsWith('coding_stage_gate_');
        if (isStageGate) {
          writeLog({ event: 'stage_gate_observed', gate_id: gate.gate_id, expires_at: gate.expires_at ?? null });
          return;
        }
        if (gate?.kind === 'blocked') {
          writeLog({ event: 'blocked_gate_required', gate_id: gate.gate_id, title: gate.title ?? null });
          if (!state.respondedGates.has(gate.gate_id) && state.gateReleases < MAX_GATE_RELEASES) {
            state.respondedGates.add(gate.gate_id);
            state.gateReleases += 1;
            writeLog({ event: 'gate_triage_release', gate_id: gate.gate_id, action: 'retry_coding', release_no: state.gateReleases });
            send({ type: 'gate_response', gate_id: gate.gate_id, action_id: 'retry_coding', extra_context: null });
          }
          return;
        }
        writeLog({ event: 'unknown_gate_observed', gate });
        return;
      }
      case 'coding_execution_event':
        collectUsage(message);
        return;
      case 'code_review_complete':
        state.reviewEvents.push({ type, at: new Date().toISOString(), verdict: message.report?.verdict ?? null });
        writeLog({ event: 'code_review_complete', verdict: message.report?.verdict ?? null });
        return;
      case 'internal_pr_review_complete':
        state.reviewEvents.push({ type, at: new Date().toISOString() });
        writeLog({ event: 'internal_pr_review_complete' });
        return;
      case 'review_request_update':
        if (message.review_request?.url) writeLog({ event: 'review_request_url', url: message.review_request.url });
        return;
      case 'coding_protocol_error':
        writeLog({ event: 'coding_protocol_error', code: message.code ?? null, message: (message.message ?? '').slice(0, 200) });
        return;
      case 'coding_recover_failed':
        writeLog({ event: 'coding_recover_failed', code: message.code ?? null });
        return;
      case 'coding_timeline_node_created':
      case 'coding_timeline_node_updated':
      case 'coding_stream_chunk':
      case 'coding_message_complete':
      case 'coding_choice_response_ack':
      case 'coding_chat_entry_created':
      case 'coding_provider_config_updated':
      case 'coding_pong':
      case 'code_review_report':
      case 'internal_pr_review':
        return;
      default:
        state.unknownTypes.add(String(type));
        writeLog({ event: 'unknown_message_type', type: String(type) });
        return;
    }
  };
  ws.onerror = () => writeLog({ event: 'ws_error' });
  ws.onclose = () => {
    writeLog({ event: 'ws_close', last_status: state.status });
    if (ended) return;
    if (TERMINAL.has(state.status ?? '')) {
      finish(state.status === 'completed' ? 0 : 1, `attempt terminal ${state.status}`);
      return;
    }
    reconnects += 1;
    setTimeout(connect, 1_500);
  };
}
connect();
setInterval(() => writeLog({ event: 'drive_alive', status: state.status, stage: state.stage, wi: state.wi, releases: state.gateReleases, recovers: state.recovers }), 60_000);
