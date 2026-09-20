// wave4-passive-ppstress：pre-provider 死因压力测试（后台）。
// 模式：复用 coding-drive.cjs 的 wire 协议，连续建 10 个 coding attempt
// （provider=fake——不真编码，只触发 runner 生命周期）：
//   建(HTTP) → WS attach + start_coding → 观察 coding stage gate →
//   gate 期内 provider_select(coder,fake) → 等 5s 倒计时自然过期（auto-continue）→
//   runner 穿过 gate 后 provider_for(Fake) 在生产 registry（无 Fake）必然 NotFound →
//   若 attempt 停留 Running：F-14 fail-closed 转 AwaitingManualRecovery，
//   F-16 双通道落死因（eprintln [aria-runner-death] → 服务器 tty；durable
//   chat-entries 尾帧 coding_manual_recovery_diagnostic_*）。
// 每个 attempt 结束后 HTTP abort 释放 work item，串行跑下一个。
// 用法: node ppstress-drive.cjs <attempts=10>
'use strict';
const fs = require('node:fs');
const path = require('node:path');

const BASE = (process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317').replace(/\/$/, '');
const WS_BASE = BASE.replace(/^http/, 'ws');
const PROJECT_ID = 'project_0001';
const REPOSITORY_ID = 'repository_0001';
const RUNS = Number(process.argv[2] ?? 10);
const HERE = __dirname;
// HERE = <repo>/cadence/reports/provider-validation-round/wave4-ppstress → 上溯 4 层到仓库根
const W = path.resolve(HERE, '..', '..', '..', '..');
const ARIA = path.join(W, '.aria');
const STAMP = new Date().toISOString().replace(/[-:.TZ]/g, '');
const LOG = path.join(HERE, `ppstress-${STAMP}.jsonl`);
const PROVIDER = 'fake';

const log = fs.createWriteStream(LOG, { flags: 'wx' });
const writeLog = (entry) => log.write(`${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function http(method, url, body) {
  const res = await fetch(url, {
    method,
    headers: body ? { 'content-type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(60_000),
  });
  const text = await res.text();
  let json = null;
  try { json = text ? JSON.parse(text) : null; } catch { json = { raw: text.slice(0, 500) }; }
  return { status: res.status, ok: res.ok, json };
}

function readAttemptFile(issueId, attemptId) {
  if (!issueId || !attemptId) return null;
  const p = path.join(ARIA, 'projects', PROJECT_ID, 'issues', issueId, 'coding-attempts', `${attemptId}.json`);
  try { return JSON.parse(fs.readFileSync(p, 'utf8')); } catch { return null; }
}

function readDeathDiagnostic(issueId, attemptId) {
  if (!issueId || !attemptId) return [];
  const dir = path.join(ARIA, 'projects', PROJECT_ID, 'issues', issueId, 'coding-attempts', attemptId, 'chat-entries');
  try {
    const files = fs.readdirSync(dir).filter((f) => f.startsWith('coding_manual_recovery_diagnostic'));
    return files.map((f) => JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8')));
  } catch { return []; }
}

// —— setup：建 issue + 手工 seed confirmed work item（legacy 单仓路由）——
async function setup() {
  const issue = await http('POST', `${BASE}/api/projects/${PROJECT_ID}/issues`, {
    title: `wave4-passive-ppstress pre-provider 死因压测 ${STAMP}`,
    description: '压力测试 issue：连续 10 个 coding attempt 触发 runner 生命周期（provider=fake），观察 pre-provider 死亡与 F-16 死因通道。非交付 issue。',
    repository_id: REPOSITORY_ID,
  });
  if (!issue.ok) throw new Error(`create issue HTTP ${issue.status}: ${JSON.stringify(issue.json).slice(0, 300)}`);
  const issueId = issue.json.issue_id ?? issue.json.issue?.issue_id ?? issue.json.id;
  if (!issueId) throw new Error(`create issue 响应缺 issue_id: ${JSON.stringify(issue.json).slice(0, 300)}`);
  const now = new Date().toISOString();
  const workItem = {
    id: 'work_item_0001',
    project_id: PROJECT_ID,
    issue_id: issueId,
    repository_id: REPOSITORY_ID,
    target_repository_id: null,
    story_spec_ids: [],
    design_spec_ids: [],
    title: 'ppstress 占位 work item（不产出代码）',
    plan_status: 'confirmed',
    execution_status: 'pending',
    worktree_path: null,
    kind: 'other',
    depends_on: [],
    exclusive_write_scopes: [],
    forbidden_write_scopes: [],
    require_execution_plan_confirm: false,
    execution_plan_status: 'not_started',
    created_at: now,
    updated_at: now,
  };
  const wiPath = path.join(ARIA, 'projects', PROJECT_ID, 'issues', issueId, 'work-items', 'work_item_0001.json');
  fs.mkdirSync(path.dirname(wiPath), { recursive: true });
  fs.writeFileSync(wiPath, JSON.stringify(workItem, null, 2));
  writeLog({ event: 'setup_done', issue_id: issueId, work_item: 'work_item_0001' });
  return { issueId, workItemId: 'work_item_0001' };
}

// —— 单个 attempt 全生命周期 ——
function runAttempt(k, issueId, workItemId) {
  return new Promise(async (resolve) => {
    const rec = {
      run: k,
      attempt_id: null,
      t_create: new Date().toISOString(),
      t_gate: null,
      t_provider_select: null,
      t_gate_expired: null,
      t_end: null,
      provider_select_ack: null,
      status_timeline: [],
      ws_events: [],
      outcome: null,           // death_amr | no_death_* | error_*
      final_status: null,
      manual_recovery_reason: null,
      death_failure_detail: null,   // F-16 durable 死因串
      aborted: false,
    };
    const pushStatus = (status, stage) => {
      const last = rec.status_timeline.at(-1);
      if (!last || last.status !== status || last.stage !== stage) {
        rec.status_timeline.push({ status, stage, at: new Date().toISOString() });
      }
    };
    const wsEvents = (name, extra) => {
      rec.ws_events.push({ name, at: new Date().toISOString(), ...extra });
      writeLog({ run: k, event: name, ...extra });
    };

    // 1. 建 attempt
    const created = await http('POST',
      `${BASE}/api/projects/${PROJECT_ID}/issues/${issueId}/work-items/${workItemId}/coding-attempts`);
    if (!created.ok) {
      rec.outcome = 'error_create_failed';
      rec.error = `HTTP ${created.status}: ${JSON.stringify(created.json).slice(0, 300)}`;
      rec.t_end = new Date().toISOString();
      writeLog({ run: k, event: 'create_failed', error: rec.error });
      return resolve(rec);
    }
    rec.attempt_id = created.json.id ?? created.json.attempt?.id ?? created.json.attempt_id;
    rec.initial_status = created.json.status ?? null;
    if (!rec.attempt_id) {
      rec.outcome = 'error_create_response';
      rec.error = JSON.stringify(created.json).slice(0, 300);
      return resolve(rec);
    }
    writeLog({ run: k, event: 'attempt_created', attempt_id: rec.attempt_id, dto_status: rec.initial_status });

    // 2. WS attach + 驱动
    const url = `${WS_BASE}/ws/projects/${PROJECT_ID}/issues/${issueId}/coding-attempts/${rec.attempt_id}`;
    const ws = new WebSocket(url);
    let settled = false;
    let gateSeen = false;
    let providerSent = false;
    let abortFallback = false;

    const finish = async (outcome) => {
      if (settled) return;
      settled = true;
      rec.outcome = outcome;
      rec.t_end = new Date().toISOString();
      try { ws.close(); } catch { /* noop */ }
      resolve(rec);
    };

    // 总超时：90s（worktree 首建 + gate 5s + provider 窗口 + AMR 转换）
    const hardTimer = setTimeout(() => finish('error_timeout_90s'), 90_000);

    // gate 后观察窗：gate 过期后 30s 内无 AMR → 判定未死亡，abort 收尾
    let postGateWatch = null;

    ws.onopen = () => {
      wsEvents('ws_open');
      ws.send(JSON.stringify({ type: 'coding_hello', attempt_id: rec.attempt_id, last_seen_node_id: null }));
      setTimeout(() => ws.send(JSON.stringify({ type: 'start_coding' })), 200);
    };

    ws.onmessage = (event) => {
      let msg = null;
      try { msg = JSON.parse(String(event.data)); } catch { return; }
      const type = msg.type ?? 'unknown';
      switch (type) {
        case 'coding_session_state':
          pushStatus(msg.status ?? null, msg.stage ?? null);
          wsEvents('session_state', { status: msg.status, stage: msg.stage });
          if (msg.status === 'awaiting_manual_recovery') {
            rec.final_status = msg.status;
            rec.manual_recovery_reason = msg.manual_recovery_reason ?? null;
            clearTimeout(hardTimer);
            clearTimeout(postGateWatch);
            setTimeout(() => finish('death_amr'), 500);
          }
          return;
        case 'coding_stage_change':
          wsEvents('stage_change', { stage: msg.stage });
          return;
        case 'coding_gate_required': {
          const gate = msg.gate ?? msg;
          const isStageGate = gate?.kind === 'stage_gate' || String(gate?.gate_id ?? '').startsWith('coding_stage_gate_');
          wsEvents('gate_required', { gate_id: gate?.gate_id, kind: gate?.kind, stage: gate?.stage, isStageGate });
          if (isStageGate && !gateSeen && !providerSent) {
            gateSeen = true;
            rec.t_gate = new Date().toISOString();
            // gate 倒计时期内换 coder provider → fake（runner 期内应用并落盘）
            providerSent = true;
            rec.t_provider_select = new Date().toISOString();
            ws.send(JSON.stringify({ type: 'provider_select', role: 'coder', provider: PROVIDER }));
            wsEvents('provider_select_sent', { role: 'coder', provider: PROVIDER });
            // gate 5s 过期 + 观察窗
            postGateWatch = setTimeout(() => finish('no_death_window_elapsed'), 5_000 + 30_000);
          }
          return;
        }
        case 'coding_execution_event': {
          const title = msg.event?.title ?? null;
          if (title === 'stage_gate_auto_continue') {
            rec.t_gate_expired = new Date().toISOString();
            wsEvents('stage_gate_auto_continue', {});
          }
          return;
        }
        case 'coding_provider_config_updated':
          rec.provider_select_ack = { role: msg.role ?? 'coder', provider: msg.provider ?? PROVIDER, at: new Date().toISOString() };
          wsEvents('provider_config_updated', { role: msg.role, provider: msg.provider });
          return;
        case 'coding_protocol_error':
          wsEvents('protocol_error', { code: msg.code ?? null, message: (msg.message ?? '').slice(0, 300) });
          // provider_select 被拒（role locked / select failed）→ 真实 provider 将启动 → 立即中止保安全
          if (msg.code === 'coding_provider_role_locked' || msg.code === 'coding_provider_select_failed') {
            if (!abortFallback) {
              abortFallback = true;
              wsEvents('provider_select_rejected_abort', { code: msg.code });
              clearTimeout(hardTimer);
              clearTimeout(postGateWatch);
              setTimeout(() => finish('error_provider_select_rejected'), 300);
            }
          }
          return;
        case 'coding_pong':
        case 'coding_timeline_node_created':
        case 'coding_timeline_node_updated':
        case 'coding_stream_chunk':
        case 'coding_message_complete':
        case 'coding_chat_entry_created':
          return;
        default:
          wsEvents('unhandled_message', { type: String(type) });
          return;
      }
    };
    ws.onerror = () => wsEvents('ws_error');
    ws.onclose = (e) => {
      wsEvents('ws_close', { code: e?.code });
      // AMR 已判死则由 finish 收口；否则等 2s 落盘状态后按未复现处理
      if (!settled) {
        setTimeout(() => finish('no_death_ws_closed'), 2_000);
      }
    };
  });
}

async function main() {
  const summary = {
    started_at: new Date().toISOString(),
    base_url: BASE,
    server_pid: 3305925,
    provider: PROVIDER,
    runs_requested: RUNS,
    setup: null,
    runs: [],
    death_count: 0,
    no_death_count: 0,
    error_count: 0,
  };
  const { issueId, workItemId } = await setup();
  summary.setup = { issue_id: issueId, work_item_id: workItemId };

  for (let k = 1; k <= RUNS; k += 1) {
    writeLog({ run: k, event: 'run_start' });
    const rec = await runAttempt(k, issueId, workItemId);
    writeLog({ run: k, event: 'run_ws_phase_done', outcome: rec.outcome, final_status: rec.final_status });

    // durable 层取证：attempt json + chat-entries 死因尾帧
    const attemptFile = readAttemptFile(issueId, rec.attempt_id);
    if (attemptFile) {
      rec.final_status = attemptFile.status ?? rec.final_status;
      rec.manual_recovery_reason = attemptFile.manual_recovery_reason ?? rec.manual_recovery_reason;
    }
    const diagnostics = readDeathDiagnostic(issueId, rec.attempt_id);
    if (diagnostics.length > 0) {
      const d = diagnostics.at(-1);
      rec.death_failure_detail = d?.metadata?.failure_detail ?? d?.entry_type?.message ?? null;
      rec.death_diagnostic_entry_id = d?.id ?? null;
    }
    if (rec.outcome === 'death_amr' || rec.final_status === 'awaiting_manual_recovery') {
      rec.outcome = rec.outcome === 'death_amr' ? 'death_amr' : 'death_amr_file_status';
    }

    // abort 收尾（AMR abort-only；未死亡/异常也 abort 防真实 provider 继续跑）
    if (rec.attempt_id && rec.final_status !== 'aborted' && rec.final_status !== 'completed' && rec.final_status !== 'failed') {
      const aborted = await http('POST', `${BASE}/api/coding-attempts/${rec.attempt_id}/abort`);
      rec.abort_http_status = aborted.status;
      rec.aborted = aborted.ok && (aborted.json?.status === 'aborted');
      // abort 后复核（engine 终态转换有延迟容忍）
      for (let i = 0; i < 10 && !rec.aborted; i += 1) {
        await sleep(500);
        const check = await http('GET', `${BASE}/api/coding-attempts/${rec.attempt_id}`);
        rec.aborted = check.json?.status === 'aborted';
        if (rec.aborted) rec.abort_http_status = check.status;
      }
    } else if (rec.attempt_id) {
      rec.aborted = true;
    }
    rec.t_abort_done = new Date().toISOString();
    writeLog({ run: k, event: 'run_done', outcome: rec.outcome, final_status: rec.final_status, aborted: rec.aborted, death_detail: rec.death_failure_detail });

    if (rec.outcome?.startsWith('death_')) summary.death_count += 1;
    else if (rec.outcome?.startsWith('no_death_')) summary.no_death_count += 1;
    else summary.error_count += 1;
    summary.runs.push(rec);
    // attempt 间静置 1s，规避锁残留
    await sleep(1_000);
  }

  summary.completed_at = new Date().toISOString();
  const outPath = path.join(HERE, `ppstress-summary-${STAMP}.json`);
  fs.writeFileSync(outPath, JSON.stringify(summary, null, 2));
  console.log(`SUMMARY=${outPath}`);
  console.log(JSON.stringify({
    issue_id: issueId,
    runs: RUNS,
    deaths: summary.death_count,
    no_death: summary.no_death_count,
    errors: summary.error_count,
  }, null, 2));
  log.end(() => process.exit(0));
}

main().catch((e) => {
  writeLog({ event: 'fatal', error: String(e?.stack ?? e) });
  console.error(e);
  log.end(() => process.exit(1));
});
