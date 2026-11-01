// wave2-2c story terminate 驱动 v2：接手现有 story 会话（0472），应答 pending choice → 等门 → abandon_human_gate → 观察 → confirm 追证。
// 用法: node story_terminate_v2.mjs <sessionId> <issueId>
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..', '..', '..', '..');
const ARIA_ROOT = path.join(REPO_ROOT, '.aria');
const BASE = 'http://127.0.0.1:4317';
const WS_BASE = BASE.replace(/^http/, 'ws');
const PROJECT_ID = 'project_0001';
const SESSION_ID = process.argv[2];
const ISSUE_ID = process.argv[3];
const GATE_WAIT_MS = 20 * 60_000;
const OBSERVE_MS = 30_000;

const now = () => new Date().toISOString();
const mono = () => Number(process.hrtime.bigint() / 1_000n) / 1_000;

function wsLog(entry) {
  fs.appendFileSync(path.join(HERE, 'story-terminate-ws.jsonl'), JSON.stringify({ t: now(), driver: 'v2', ...entry }) + '\n');
}

const result = {
  started_at: now(),
  session_id: SESSION_ID,
  issue_id: ISSUE_ID,
  choices_answered: [],
  gate_stage_seen: null,
  gate_active_node: null,
  t_gate_reached: null,
  abandon: { command_id: null, sent_at: null, t0: null, responses: [], session_status_after: null, session_stage_after: null, outcome: null },
  confirm_followup: { sent_at: null, t0: null, responses: [], session_status_after: null, outcome: null },
  stage_timeline: [],
  outcome: null,
};

function readSession() {
  try {
    return JSON.parse(fs.readFileSync(path.join(ARIA_ROOT, 'projects', PROJECT_ID, 'issues', ISSUE_ID, 'workspace-sessions', `${SESSION_ID}.json`), 'utf8'));
  } catch {
    return null;
  }
}

function sendChoiceAnswer(ws, msg) {
  // 首选项（含「推荐」优先，否则首项）——保持流程继续
  const options = msg.options ?? [];
  let selected = options.find((o) => String(o.label ?? '').includes('推荐'));
  if (!selected) selected = options[0];
  const answer = {
    question_id: null,
    selected_option_ids: selected?.id ? [selected.id] : [],
    free_text: null,
  };
  const payload = {
    type: 'choice_response',
    id: msg.id,
    selected_option_ids: selected?.id ? [selected.id] : [],
    free_text: null,
    answers: (msg.questions ?? []).map((q) => ({
      question_id: q.id ?? q.question_id ?? null,
      selected_option_ids: q.options?.[0]?.id ? [q.options[0].id] : (selected?.id ? [selected.id] : []),
      free_text: null,
    })),
  };
  result.choices_answered.push({ id: msg.id, selected: selected?.label ?? selected ?? null, at: now() });
  wsLog({ event: 'choice_response_sent', request_id: msg.id, selected_label: selected?.label ?? String(selected) });
  ws.send(JSON.stringify(payload));
}

async function main() {
  const ws = new WebSocket(`${WS_BASE}/api/workspace-sessions/${encodeURIComponent(SESSION_ID)}/ws`);
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

  ws.onmessage = (event) => {
    let msg = null;
    try { msg = JSON.parse(event.data); } catch { return; }
    const type = String(msg?.type);
    if (type === 'session_state') {
      const stage = msg.stage;
      if (stage && (result.stage_timeline.length === 0 || result.stage_timeline[result.stage_timeline.length - 1] !== stage)) {
        result.stage_timeline.push(stage);
      }
      if (!gateReached && (stage === 'human_confirm' || stage === 'author_confirm')) {
        gateReached = true;
        result.t_gate_reached = now();
        result.gate_stage_seen = stage;
        result.gate_active_node = msg.active_node_id ?? null;
        wsLog({ event: 'gate_reached', stage, active_node_id: result.gate_active_node });
        setTimeout(() => {
          const commandId = `cmd_wave2c_abandon_${Date.now()}`;
          result.abandon.command_id = commandId;
          result.abandon.sent_at = now();
          result.abandon.t0 = mono();
          abandonSent = true;
          wsLog({ event: 'abandon_human_gate_sent', command_id: commandId });
          ws.send(JSON.stringify({ type: 'abandon_human_gate', command_id: commandId }));
        }, 1000);
      }
      return;
    }
    if (type === 'choice_request') {
      sendChoiceAnswer(ws, msg);
      return;
    }
    if (type === 'stage_change') {
      wsLog({ event: 'stage_change', stage: msg.stage ?? msg.to });
    }
    if (abandonSent && !confirmSent && result.abandon.responses.length < 60) {
      result.abandon.responses.push({ t: now(), delta_ms: Math.round(mono() - result.abandon.t0), msg });
      if (type === 'error' || type === 'protocol_error') {
        result.abandon.outcome ??= `${type}: ${msg.code ?? ''} ${msg.message ?? ''}`.trim();
        wsLog({ event: 'abandon_response', type, code: msg.code ?? null, message: msg.message ?? null });
      }
    } else if (confirmSent && result.confirm_followup.responses.length < 60) {
      result.confirm_followup.responses.push({ t: now(), delta_ms: Math.round(mono() - result.confirm_followup.t0), msg });
      if (type === 'error' || type === 'protocol_error') {
        result.confirm_followup.outcome ??= `${type}: ${msg.code ?? ''} ${msg.message ?? ''}`.trim();
        wsLog({ event: 'confirm_response', type, code: msg.code ?? null, message: msg.message ?? null });
      }
    }
  };
  ws.onopen = () => {
    wsLog({ event: 'ws_open_v2' });
    ws.send(JSON.stringify({ type: 'hello', session_id: SESSION_ID, last_seen_node_id: null }));
  };
  ws.onerror = (e) => wsLog({ event: 'ws_error', message: e?.message ?? null });
  ws.onclose = (e) => wsLog({ event: 'ws_close', code: e?.code, reason: e?.reason });

  await new Promise((resolve) => {
    const check = setInterval(() => {
      if (done) { clearInterval(check); resolve(); return; }
      if (!gateReached && Date.now() - Date.parse(result.started_at) > GATE_WAIT_MS) {
        result.gate_unreachable = true;
        finish('gate_unreachable_timeout');
        return;
      }
      if (result.abandon.sent_at && Date.now() - Date.parse(result.abandon.sent_at) > OBSERVE_MS && !confirmSent) {
        const sess = readSession();
        result.abandon.session_status_after = sess?.status ?? null;
        result.abandon.session_stage_after = sess?.stage ?? null;
        const closed = ['abandoned', 'terminated'].includes(String(sess?.status));
        result.abandon.outcome ??= closed ? 'gate_closed_abandoned' : `no_close(session=${sess?.status ?? 'unknown'})`;
        wsLog({ event: 'abandon_observed', status: sess?.status, stage: sess?.stage, outcome: result.abandon.outcome });
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
          }, 20_000);
        } else {
          finish('story_terminate_observed');
        }
      }
    }, 250);
  });
}

try {
  await main();
} catch (error) {
  result.fatal = String(error?.stack ?? error);
  result.outcome ??= 'driver_error';
}
result.completed_at = now();
fs.writeFileSync(path.join(HERE, 'story-terminate-v2-evidence.json'), JSON.stringify(result, null, 2));
console.log(JSON.stringify({
  outcome: result.outcome,
  gate_stage: result.gate_stage_seen,
  abandon_outcome: result.abandon.outcome,
  abandon_session_after: result.abandon.session_status_after,
  confirm_outcome: result.confirm_followup.outcome,
  choices: result.choices_answered.length,
  fatal: result.fatal ?? null,
}, null, 2));
process.exit(0);
