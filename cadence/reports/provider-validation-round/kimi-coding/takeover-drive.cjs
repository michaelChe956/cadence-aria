// 2b 手工 typed 驱动：takeover 子会话（interactive SC）门应答 + Confirmed 后 advance。
// 动机：campaign 驱动的 stage3 门控以 active timeline node 为触发（takeover 子会话
// 时间线为空、active_node_id=None），无法应答 attach 前已开的 human_confirm 门；
// 本脚本按 protocol.rs 准入表（SC+HumanConfirm 收 confirm|human_gate_feedback|
// abandon_human_gate）直接发 bare confirm（inbound.rs:189 → SC 分支 =
// HumanGateCloseDecision::Approve），Confirmed 后发 typed advance（advance_completed
// → coding attempt，provider 从会话冻结 kimi）。
// 用法：node takeover-drive.cjs <session_id> <outDir>
'use strict';
const { randomUUID } = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const BASE = (process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317').replace(/\/$/, '');
const WS_BASE = (process.env.ARIA_WS_BASE_URL ?? BASE.replace(/^http/, 'ws')).replace(/\/$/, '');
const HARD_LIMIT_MS = Number(process.env.ARIA_TAKEOVER_DRIVE_HARD_TIMEOUT_MS ?? 20 * 60_000);
const [sessionId, outDir] = process.argv.slice(2);
if (!sessionId || !outDir) {
  console.error('Usage: node takeover-drive.cjs <session_id> <outDir>');
  process.exit(2);
}
fs.mkdirSync(outDir, { recursive: true });
const log = fs.createWriteStream(path.join(outDir, 'takeover-drive-ws.jsonl'), { flags: 'wx' });
const writeLog = (entry) => log.write(`${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`);

const advanceCommandId = `advance_${randomUUID()}`;
let ws = null;
let ended = false;
let confirmedSeen = false;
let advanceSent = false;
let confirmCount = 0;
const started = Date.now();

const hardTimer = setTimeout(() => finish(3, `takeover-drive 硬超时 ${HARD_LIMIT_MS}ms`), HARD_LIMIT_MS);

function finish(code, reason) {
  if (ended) return;
  ended = true;
  clearTimeout(hardTimer);
  writeLog({ event: 'finish', code, reason: reason ?? null, confirm_count: confirmCount, confirmed_seen: confirmedSeen, advance_sent: advanceSent });
  try { ws?.close(); } catch { /* noop */ }
  log.end(() => process.exit(code));
}

function connect() {
  const url = `${WS_BASE}/api/workspace-sessions/${encodeURIComponent(sessionId)}/ws`;
  ws = new WebSocket(url);
  ws.onopen = () => {
    writeLog({ event: 'ws_open', url });
    ws.send(JSON.stringify({ type: 'hello', session_id: sessionId, last_seen_node_id: null }));
  };
  ws.onmessage = (event) => {
    let message;
    try { message = JSON.parse(String(event.data)); } catch {
      writeLog({ direction: 'in', raw: String(event.data).slice(0, 400) });
      return;
    }
    writeLog({ direction: 'in', message });
    handle(message);
  };
  ws.onerror = () => writeLog({ event: 'ws_error' });
  ws.onclose = (event) => {
    writeLog({ event: 'ws_close', code: event.code, wasClean: event.wasClean });
    if (ended) return;
    // 服务端 idle-timeout（~90s）会关空连接：重连以 durable 状态续判。
    setTimeout(connect, 2_000);
  };
}

function send(message) {
  writeLog({ direction: 'out', message });
  ws.send(JSON.stringify(message));
}

function handle(message) {
  const type = message.type;
  if (type === 'session_state') {
    const status = message.session_status;
    const stage = message.stage;
    if (status === 'confirmed') {
      confirmedSeen = true;
      if (!advanceSent) {
        advanceSent = true;
        writeLog({ event: 'advance_send', command_id: advanceCommandId });
        send({ type: 'advance', command_id: advanceCommandId });
      }
      return;
    }
    if (status === 'waiting_for_human' && stage === 'human_confirm') {
      // 幂等护栏：仅在未发过 advance 时应答门；confirm 计数仅观测。
      if (!advanceSent) {
        confirmCount += 1;
        writeLog({ event: 'confirm_send', gate_no: confirmCount });
        send({ type: 'confirm' });
      }
      return;
    }
    if (['failed', 'aborted', 'stopped_needs_human'].includes(status)) {
      finish(1, `session 终态 ${status}`);
      return;
    }
    return;
  }
  if (type === 'advance_completed') {
    fs.writeFileSync(path.join(outDir, 'advance-result.json'), JSON.stringify({
      session_id: sessionId,
      command_id: advanceCommandId,
      completed: true,
      attempt_id: message.attempt_id ?? null,
      record: message.record ?? null,
    }, null, 2));
    writeLog({ event: 'advance_completed', attempt_id: message.attempt_id, command_id: message.command_id });
    finish(0, 'advance_completed');
    return;
  }
  if (type === 'advance_rejected') {
    fs.writeFileSync(path.join(outDir, 'advance-result.json'), JSON.stringify({
      session_id: sessionId,
      command_id: advanceCommandId,
      completed: false,
      code: message.code ?? null,
      reason: message.reason ?? null,
    }, null, 2));
    writeLog({ event: 'advance_rejected', code: message.code, reason: message.reason });
    finish(1, `advance_rejected: ${message.code}`);
    return;
  }
  if (type === 'error') {
    writeLog({ event: 'server_error', message: message.message ?? null });
    // error 帧不直接退出：由 durable session_state 判终态。
    return;
  }
}

connect();
