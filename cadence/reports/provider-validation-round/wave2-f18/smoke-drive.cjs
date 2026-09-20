// F-18 smoke：WS typed abandon 关 story author_confirm 门 + HTTP confirm 双面验证。
const BASE = 'http://127.0.0.1:4399';
const WS_BASE = BASE.replace('http', 'ws');
const SID = 'workspace_session_0472';
const CONFIRM_SID = 'workspace_session_0473';
const SESSION_FILE = '/tmp/f18-smoke/.aria/projects/project_0001/issues/issue_0301/workspace-sessions';
const fs = require('fs');

const now = () => new Date().toISOString();
const log = (m) => console.log(`[${now()}] ${m}`);
const readStatus = (sid) => JSON.parse(fs.readFileSync(`${SESSION_FILE}/${sid}.json`, 'utf8')).status;

const result = { started_at: now() };

async function httpFace() {
  const res = await fetch(`${BASE}/api/workspace-sessions/${CONFIRM_SID}/confirm`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ confirmed_by: 'f18-smoke' }),
  });
  const body = await res.json().catch(() => null);
  result.http_confirm = {
    status: res.status,
    session_status_after: readStatus(CONFIRM_SID),
    dto_status: body?.status ?? null,
  };
  log(`HTTP confirm: ${res.status} durable=${result.http_confirm.session_status_after}`);
}

function wsFace() {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`${WS_BASE}/api/workspace-sessions/${SID}/ws`);
    const timer = setTimeout(() => reject(new Error('smoke timeout')), 30_000);
    let stageSeen = null;
    let abandonSent = false;
    ws.onopen = () => {
      log('ws open, hello');
      ws.send(JSON.stringify({ type: 'hello', session_id: SID, last_seen_node_id: null }));
    };
    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.type === 'session_state') {
        stageSeen ??= msg.stage;
        log(`session_state stage=${msg.stage} status=${msg.session_status ?? ''}`);
        if (msg.stage === 'author_confirm' && !abandonSent) {
          abandonSent = true;
          setTimeout(() => {
            log('send abandon_human_gate');
            ws.send(JSON.stringify({ type: 'abandon_human_gate', command_id: 'cmd_f18_smoke_1' }));
          }, 300);
        }
      } else if (msg.type === 'human_gate_closed') {
        log(`human_gate_closed decision=${msg.decision} stage=${msg.stage}`);
        result.ws_abandon = {
          gate_stage_before: stageSeen,
          close_event: { decision: msg.decision, stage: msg.stage },
          session_status_after: readStatus(SID),
        };
        setTimeout(() => {
          result.ws_abandon.session_status_after = readStatus(SID);
          clearTimeout(timer);
          ws.close();
          resolve();
        }, 300);
      } else if (msg.type === 'error' || msg.type === 'protocol_error') {
        log(`OUT-OF-EXPECTATION ${msg.type}: ${msg.code ?? ''} ${msg.message ?? ''}`);
        result.ws_abandon = { error: `${msg.type}: ${msg.code ?? ''} ${msg.message ?? ''}`, session_status_after: readStatus(SID) };
        setTimeout(() => {
          result.ws_abandon.session_status_after = readStatus(SID);
          clearTimeout(timer);
          ws.close();
          resolve();
        }, 300);
      }
    };
    ws.onerror = (e) => { clearTimeout(timer); reject(new Error(`ws error: ${e.message}`)); };
  });
}

(async () => {
  await httpFace();
  await wsFace();
  result.completed_at = now();
  fs.writeFileSync('/tmp/f18-smoke/result.json', JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
  process.exit(0);
})().catch((e) => { console.error(e); process.exit(1); });
