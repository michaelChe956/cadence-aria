// 2b 人工分诊放行：coding blocked gate → gate_response{action_id}（retry_coding）。
// 用法：node gate-release.cjs <project_id> <issue_id> <attempt_id> <gate_id> <action_id> <outDir>
'use strict';
const fs = require('node:fs');
const path = require('node:path');

const BASE = (process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317').replace(/\/$/, '');
const WS_BASE = (process.env.ARIA_WS_BASE_URL ?? BASE.replace(/^http/, 'ws')).replace(/\/$/, '');
const HARD_LIMIT_MS = Number(process.env.ARIA_GATE_RELEASE_HARD_TIMEOUT_MS ?? 3 * 60_000);
const [projectId, issueId, attemptId, gateId, actionId, outDir] = process.argv.slice(2);
if (![projectId, issueId, attemptId, gateId, actionId, outDir].every((v) => v)) {
  console.error('Usage: node gate-release.cjs <project_id> <issue_id> <attempt_id> <gate_id> <action_id> <outDir>');
  process.exit(2);
}
fs.mkdirSync(outDir, { recursive: true });
const log = fs.createWriteStream(path.join(outDir, `gate-release-${gateId}-${Date.now()}.jsonl`), { flags: 'wx' });
const writeLog = (entry) => log.write(`${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`);

const url = `${WS_BASE}/ws/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/coding-attempts/${encodeURIComponent(attemptId)}`;
const ws = new WebSocket(url);
let released = false;
let closed = false;
const hardTimer = setTimeout(() => finish(released ? 0 : 3, `gate-release 硬超时 ${HARD_LIMIT_MS}ms`), HARD_LIMIT_MS);

function finish(code, reason) {
  clearTimeout(hardTimer);
  writeLog({ event: 'finish', code, reason: reason ?? null, released, gate_closed: closed });
  try { ws.close(); } catch { /* noop */ }
  log.end(() => process.exit(code));
}

ws.onopen = () => {
  writeLog({ event: 'ws_open', url });
  ws.send(JSON.stringify({ type: 'coding_hello', attempt_id: attemptId, last_seen_node_id: null }));
};
ws.onmessage = (event) => {
  let message;
  try { message = JSON.parse(String(event.data)); } catch { return; }
  writeLog({ direction: 'in', message });
  const type = message.type;
  if (type === 'coding_session_state' && !released) {
    const pending = (message.pending_gates || []).some((g) => g.gate_id === gateId);
    if (pending || message.status === 'blocked') {
      released = true;
      const outbound = { type: 'gate_response', gate_id: gateId, action_id: actionId, extra_context: null };
      writeLog({ direction: 'out', message: outbound });
      ws.send(JSON.stringify(outbound));
    }
    return;
  }
  if (type === 'coding_gate_closed' && message.gate_id === gateId) {
    closed = true;
    writeLog({ event: 'gate_closed', gate_id: gateId });
    finish(0, 'gate_closed');
    return;
  }
  if (type === 'coding_protocol_error') {
    writeLog({ event: 'protocol_error', code: message.code });
    finish(1, `protocol_error ${message.code}`);
    return;
  }
  if (type === 'coding_session_state' && !['blocked', 'waiting_for_human'].includes(message.status) && released) {
    // 放行后进入非 blocked 态即成功（部分路径无显式 gate_closed 帧）。
    finish(0, `session status=${message.status}`);
  }
};
ws.onerror = () => { writeLog({ event: 'ws_error' }); finish(2, 'ws error'); };
ws.onclose = () => { if (!closed) finish(released ? 0 : 2, `ws close (released=${released})`); };
