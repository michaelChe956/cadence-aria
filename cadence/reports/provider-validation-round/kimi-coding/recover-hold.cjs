// 2b F-16 恢复驻留：attach coding WS → recover_coding → 保持连接（runner 事件通道
// 需要至少一个 WS 消费者；唯一消费者断开 → coding_event_channel_closed → runner 死）。
// 驻留至 attempt 终态或外部 SIGTERM。用法：
// node recover-hold.cjs <project_id> <issue_id> <attempt_id> <outDir>
'use strict';
const fs = require('node:fs');
const path = require('node:path');

const BASE = (process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317').replace(/\/$/, '');
const WS_BASE = (process.env.ARIA_WS_BASE_URL ?? BASE.replace(/^http/, 'ws')).replace(/\/$/, '');
const [projectId, issueId, attemptId, outDir] = process.argv.slice(2);
if (![projectId, issueId, attemptId, outDir].every((v) => v)) {
  console.error('Usage: node recover-hold.cjs <project_id> <issue_id> <attempt_id> <outDir>');
  process.exit(2);
}
fs.mkdirSync(outDir, { recursive: true });
const logPath = path.join(outDir, `recover-hold-${Date.now()}.jsonl`);
const log = fs.createWriteStream(logPath, { flags: 'wx' });
const writeLog = (entry) => log.write(`${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`);
console.log('LOG=' + logPath);

const TERMINAL = new Set(['completed', 'aborted', 'failed']);
let recovering = false;
let recovered = false;
let ws = null;
let lastStatus = null;
let reconnects = 0;
const MAX_RECONNECTS = 60; // 驻留期网络抖动重连上限（每次重连都会再次触发 attach 语义）

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
    const type = message.type;
    if (type === 'coding_session_state') {
      lastStatus = message.status;
      writeLog({ event: 'session_state', status: message.status, stage: message.stage, wi: message.current_work_item_id });
      if (message.status === 'awaiting_manual_recovery' && !recovered) {
        recovering = true;
        const outbound = { type: 'recover_coding' };
        writeLog({ direction: 'out', message: outbound });
        ws.send(JSON.stringify(outbound));
        return;
      }
      if (message.status === 'running') recovering = false;
      if (TERMINAL.has(message.status)) {
        writeLog({ event: 'attempt_terminal', status: message.status });
        setTimeout(() => process.exit(0), 500);
      }
      return;
    }
    if (type === 'coding_recover_failed') {
      writeLog({ event: 'coding_recover_failed', code: message.code ?? null, message: message.message ?? null });
      recovered = false;
      return;
    }
    if (type === 'coding_protocol_error') {
      writeLog({ event: 'coding_protocol_error', code: message.code ?? null, message: (message.message ?? '').slice(0, 200) });
      return;
    }
    // 其余事件（execution_event 等）量大，只记类型采样。
    if (type === 'coding_execution_event') return;
    writeLog({ event: 'observed', type });
  };
  ws.onerror = () => writeLog({ event: 'ws_error' });
  ws.onclose = () => {
    writeLog({ event: 'ws_close', last_status: lastStatus });
    if (TERMINAL.has(lastStatus ?? '')) process.exit(0);
    if (reconnects < MAX_RECONNECTS) {
      reconnects += 1;
      setTimeout(connect, 1_000);
    }
  };
}
connect();
// 每 30s 心跳记录存活（也用于外部观察本进程是否还活着）。
setInterval(() => writeLog({ event: 'hold_alive', status: lastStatus }), 30_000);
