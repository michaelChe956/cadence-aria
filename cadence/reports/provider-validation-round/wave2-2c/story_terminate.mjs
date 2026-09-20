// wave2-2c story terminate 真实链验证：story 会话到门 → 发 typed abandon_human_gate → 观察终态。
// 用法: node story_terminate.mjs
// 证据: 同目录 story-terminate-evidence.json / story-terminate-ws.jsonl
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..', '..', '..', '..');
const ARIA_ROOT = path.join(REPO_ROOT, '.aria');
const BASE = 'http://127.0.0.1:4317';
const WS_BASE = BASE.replace(/^http/, 'ws');
const PROJECT_ID = 'project_0001';
const REPOSITORY_ID = 'repository_0001';
const PROVIDER = 'pi';
const GATE_WAIT_MS = 15 * 60_000; // 15min 未到门=超时登记
const OBSERVE_MS = 30_000;

const now = () => new Date().toISOString();
const mono = () => Number(process.hrtime.bigint() / 1_000n) / 1_000;

const logLines = [];
function wsLog(entry) {
  const line = JSON.stringify({ t: now(), ...entry });
  logLines.push(line);
  fs.appendFileSync(path.join(HERE, 'story-terminate-ws.jsonl'), line + '\n');
}

async function requestJson(url, options) {
  const response = await fetch(url, { ...options, signal: AbortSignal.timeout(60_000) });
  const text = await response.text();
  let body = {};
  try { body = text ? JSON.parse(text) : {}; } catch { body = { raw: text.slice(0, 1000) }; }
  if (!response.ok) {
    throw new Error(`${options.label ?? 'request'} HTTP ${response.status}: ${typeof body.message === 'string' ? body.message : text.slice(0, 300)}`);
  }
  return body;
}

const result = {
  started_at: now(),
  provider: PROVIDER,
  issue_id: null,
  story_spec_id: null,
  workspace_session_id: null,
  session_flow_kind: null,
  session_run_policy: null,
  t_start_generation: null,
  t_gate_reached: null,
  gate_stage_seen: null,
  gate_active_node: null,
  abandon: {
    command_id: null,
    sent_at: null,
    responses: [],
    session_status_after: null,
    session_stage_after: null,
    outcome: null,
  },
  confirm_followup: {
    sent_at: null,
    responses: [],
    session_status_after: null,
    outcome: null,
  },
  stage_timeline: [],
  outcome: null,
};

function readSession() {
  try {
    return JSON.parse(fs.readFileSync(
      path.join(ARIA_ROOT, 'projects', PROJECT_ID, 'issues', result.issue_id, 'workspace-sessions', `${result.workspace_session_id}.json`),
      'utf8',
    ));
  } catch {
    return null;
  }
}

async function main() {
  // 1. 建 issue（最小需求描述）
  const issue = await requestJson(`${BASE}/api/projects/${PROJECT_ID}/issues`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      title: 'wave2-2c story terminate 验证',
      description: '需求：为示例服务新增一个健康检查端点 GET /healthz，返回 {"status":"ok"}。仅此一项，无其他范围。',
      repository_id: REPOSITORY_ID,
    }),
    label: 'create issue',
  });
  result.issue_id = issue.issue_id ?? issue.issue?.issue_id ?? issue.id;
  if (!result.issue_id) throw new Error(`create issue 响应缺 issue_id: ${JSON.stringify(issue).slice(0, 300)}`);
  wsLog({ event: 'issue_created', issue_id: result.issue_id });

  // 2. 生成 story spec 会话
  const gen = await requestJson(`${BASE}/api/projects/${PROJECT_ID}/issues/${result.issue_id}/story-specs:generate`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      title: '健康检查端点 story',
      author_provider: PROVIDER,
      reviewer_provider: PROVIDER,
      review_rounds: 1,
      superpowers_enabled: true,
      openspec_enabled: true,
    }),
    label: 'generate story spec',
  });
  const story = gen.story_specs?.[0];
  result.story_spec_id = story?.story_spec_id ?? story?.id ?? null;
  result.workspace_session_id = gen.workspace_session?.workspace_session_id ?? gen.workspace_session?.session_id ?? gen.workspace_session?.id ?? story?.workspace_session_id ?? null;
  if (!result.workspace_session_id) throw new Error(`story generate 响应缺 session: ${JSON.stringify(gen).slice(0, 400)}`);
  const sess0 = readSession();
  result.session_flow_kind = sess0?.flow_kind ?? null;
  result.session_run_policy = sess0?.run_policy ?? null;
  wsLog({ event: 'story_session_created', story_spec_id: result.story_spec_id, session_id: result.workspace_session_id, flow_kind: result.session_flow_kind, run_policy: result.session_run_policy });

  // 3. WS 驱动到门
  const ws = new WebSocket(`${WS_BASE}/api/workspace-sessions/${encodeURIComponent(result.workspace_session_id)}/ws`);
  let startSent = false;
  let abandonSent = false;
  let confirmSent = false;
  let gateReached = false;
  let done = false;

  const finish = (outcome) => {
    if (done) return;
    done = true;
    result.outcome = outcome;
    try { ws.close(); } catch { /* ignore */ }
  };

  const recordResponse = (bucket, msg) => {
    bucket.responses.push({ t: now(), delta_ms: Math.round(mono() - (bucket.t0 ?? mono())), msg });
  };

  ws.onmessage = (event) => {
    let msg = null;
    try { msg = JSON.parse(event.data); } catch { return; }
    const type = msg?.type;
    if (type === 'session_state') {
      const stage = msg.stage;
      if (stage && (result.stage_timeline.length === 0 || result.stage_timeline[result.stage_timeline.length - 1] !== stage)) {
        result.stage_timeline.push(stage);
      }
      if (stage === 'prepare_context' && !startSent) {
        startSent = true;
        result.t_start_generation = now();
        wsLog({ event: 'start_generation_sent', reviewer_enabled: false });
        ws.send(JSON.stringify({
          type: 'start_generation',
          provider_config: {
            author: PROVIDER,
            reviewer: PROVIDER,
            review_rounds: 1,
            permission_modes: { author: 'auto', reviewer: 'auto' },
          },
          reviewer_enabled: false,
        }));
        return;
      }
      // 到门判定：author_confirm / human_confirm 阶段（story 读侧重映射 author_confirm）
      if (!gateReached && (stage === 'human_confirm' || stage === 'author_confirm')) {
        gateReached = true;
        result.t_gate_reached = now();
        result.gate_stage_seen = stage;
        result.gate_active_node = msg.active_node_id ?? null;
        wsLog({ event: 'gate_reached', stage, active_node_id: result.gate_active_node });
        // —— 门上 terminate：typed abandon_human_gate（前端纪律：无活 turn 用新 command_id）——
        setTimeout(() => {
          const commandId = `cmd_wave2c_abandon_${Date.now()}`;
          result.abandon.command_id = commandId;
          result.abandon.sent_at = now();
          result.abandon.t0 = mono();
          wsLog({ event: 'abandon_human_gate_sent', command_id: commandId });
          ws.send(JSON.stringify({ type: 'abandon_human_gate', command_id: commandId }));
        }, 1200);
      }
    }
    if (type === 'stage_change') {
      wsLog({ event: 'stage_change', stage: msg.stage ?? msg.to });
    }
    // abandon 后 30s 观察窗内的所有响应
    if (abandonSent && !confirmSent && result.abandon.responses.length < 40) {
      recordResponse(result.abandon, msg);
      const t = String(type);
      if (t === 'error' || t === 'protocol_error') {
        result.abandon.outcome = `${t}: ${msg.code ?? ''} ${msg.message ?? ''}`.trim();
        wsLog({ event: 'abandon_response', type, code: msg.code ?? null, message: msg.message ?? null });
      } else if (t === 'human_gate_closed' || t === 'session_state' || t === 'stage_change' || t === 'execution_event') {
        // 记录关键事件，判定关门与否在收尾读 durable
      }
    } else if (confirmSent && result.confirm_followup.responses.length < 40) {
      recordResponse(result.confirm_followup, msg);
      const t = String(type);
      if (t === 'error' || t === 'protocol_error') {
        result.confirm_followup.outcome = `${t}: ${msg.code ?? ''} ${msg.message ?? ''}`.trim();
        wsLog({ event: 'confirm_response', type, code: msg.code ?? null, message: msg.message ?? null });
      }
    }
  };
  ws.onopen = () => {
    wsLog({ event: 'ws_open' });
    ws.send(JSON.stringify({ type: 'hello', session_id: result.workspace_session_id, last_seen_node_id: null }));
  };
  ws.onerror = (e) => wsLog({ event: 'ws_error', message: e?.message ?? null });
  ws.onclose = (e) => wsLog({ event: 'ws_close', code: e?.code, reason: e?.reason });

  // abandon 发出后置位（由 abandon.sent_at 推断）
  const abandonSentCheck = setInterval(() => {
    if (result.abandon.sent_at && !abandonSent) abandonSent = true;
  }, 100);

  // 4. abandon 观察 30s → 读 durable → 若门仍开，追加 Confirm 观察 approve 面
  const gateWatch = setInterval(() => {
    if (gateReached) { clearInterval(gateWatch); }
    if (mono() - (result.t_start_generation ? 0 : mono()) > 0 && !gateReached && Date.now() - Date.parse(result.started_at) > GATE_WAIT_MS + 120_000) {
      wsLog({ event: 'gate_wait_timeout' });
      result.outcome_timeout_to_gate = true;
      clearInterval(gateWatch);
      finish('gate_unreachable_timeout');
    }
  }, 5000);

  await new Promise((resolve) => {
    const check = setInterval(() => {
      if (done) { clearInterval(check); resolve(); }
      // 到门超时兜底
      if (!gateReached && Date.now() - Date.parse(result.started_at) > GATE_WAIT_MS + 120_000) {
        finish('gate_unreachable_timeout');
      }
      // abandon 完成（30s 观察窗过）
      if (result.abandon.sent_at && Date.now() - Date.parse(result.abandon.sent_at) > OBSERVE_MS && !confirmSent) {
        const sess = readSession();
        result.abandon.session_status_after = sess?.status ?? null;
        result.abandon.session_stage_after = sess?.stage ?? null;
        const closed = sess?.status === 'abandoned' || sess?.status === 'terminated';
        result.abandon.outcome ??= closed ? 'gate_closed_abandoned' : `no_close(session=${sess?.status ?? 'unknown'})`;
        // 若门未关（waiting_for_human），追加 Confirm 验证 approve 面可达性
        if (sess?.status === 'waiting_for_human') {
          confirmSent = true;
          result.confirm_followup.sent_at = now();
          result.confirm_followup.t0 = mono();
          wsLog({ event: 'confirm_sent_followup' });
          try { ws.send(JSON.stringify({ type: 'confirm' })); } catch { /* ignore */ }
          setTimeout(() => {
            const s2 = readSession();
            result.confirm_followup.session_status_after = s2?.status ?? null;
            result.confirm_followup.outcome ??= s2?.status === 'confirmed' ? 'confirmed' : `session=${s2?.status ?? 'unknown'}`;
            wsLog({ event: 'confirm_followup_final', status: s2?.status });
            finish('story_terminate_observed_with_confirm_followup');
          }, 15_000);
        } else {
          finish('story_terminate_observed');
        }
      }
    }, 250);
  });
  clearInterval(abandonSentCheck);
}

try {
  await main();
} catch (error) {
  result.fatal = String(error?.stack ?? error);
  result.outcome ??= 'driver_error';
}
result.completed_at = now();
fs.writeFileSync(path.join(HERE, 'story-terminate-evidence.json'), JSON.stringify(result, null, 2));
console.log(JSON.stringify({
  outcome: result.outcome,
  gate_stage: result.gate_stage_seen,
  abandon_outcome: result.abandon.outcome,
  abandon_session_after: result.abandon.session_status_after,
  confirm_outcome: result.confirm_followup.outcome,
  fatal: result.fatal ?? null,
}, null, 2));
process.exit(0);
