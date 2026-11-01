// C1-T3 补充驱动：对 rep2 已 Confirmed 的 single_candidate plan session 发送
// typed advance 命令（stage=Completed + SingleCandidate 准入，protocol.rs:170-175），
// 触发 handle_advance → ScAdvance journal（advance_provider_config 从 plan session
// 冻结 kimi provider）→ 后续 coding_run_campaign 重放收养 kimi 三角色。
// 等价于 workitem_run_campaign 阶段 3 typed 控制器的 advance 出站（:630 同款 wire）。
// 用法：node advance-confirmed-plan.mjs <session_id> <outDir>
import { randomUUID } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

const BASE = (process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317').replace(/\/$/, '');
const WS_BASE = (process.env.ARIA_WS_BASE_URL ?? BASE.replace(/^http/, 'ws')).replace(/\/$/, '');
const HARD_LIMIT_MS = Number(process.env.ARIA_ADVANCE_HARD_TIMEOUT_MS ?? 5 * 60_000);
const [sessionId, outDir] = process.argv.slice(2);
if (!sessionId || !outDir) {
  console.error('Usage: node advance-confirmed-plan.mjs <session_id> <outDir>');
  process.exit(2);
}
fs.mkdirSync(outDir, { recursive: true });
const logPath = path.join(outDir, 'advance-ws.jsonl');
const log = fs.createWriteStream(logPath, { flags: 'wx' });
const writeLog = (entry) => log.write(`${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`);

const url = `${WS_BASE}/api/workspace-sessions/${encodeURIComponent(sessionId)}/ws`;
const ws = new WebSocket(url);
const commandId = `advance_${randomUUID()}`;
const finish = (code) => { try { ws.close(); } catch { /* noop */ } log.end(() => process.exit(code)); };
const fail = (error) => {
  writeLog({ event: 'failure', error: String(error?.message ?? error) });
  finish(1);
};
const hardTimer = setTimeout(() => fail(`advance 硬超时 ${HARD_LIMIT_MS}ms`), HARD_LIMIT_MS);
let advanced = false;

ws.onopen = () => {
  writeLog({ event: 'ws_open', url });
  ws.send(JSON.stringify({ type: 'hello', session_id: sessionId, last_seen_node_id: null }));
};
ws.onmessage = (event) => {
  let message;
  try { message = JSON.parse(String(event.data)); } catch { writeLog({ direction: 'in', raw: String(event.data).slice(0, 500) }); return; }
  writeLog({ direction: 'in', message });
  if (message.type === 'hello' || message.type === 'session_state') {
    if (!advanced) {
      advanced = true;
      const outbound = { type: 'advance', command_id: commandId };
      writeLog({ direction: 'out', message: outbound });
      ws.send(JSON.stringify(outbound));
    }
    return;
  }
  if (message.type === 'advance_completed') {
    clearTimeout(hardTimer);
    writeLog({ event: 'advance_completed', attempt_id: message.attempt_id, command_id: message.command_id });
    fs.writeFileSync(path.join(outDir, 'advance-result.json'), JSON.stringify({
      session_id: sessionId, command_id: commandId, completed: true,
      attempt_id: message.attempt_id ?? null, record: message.record ?? null,
    }, null, 2));
    finish(0);
    return;
  }
  if (message.type === 'advance_rejected') {
    clearTimeout(hardTimer);
    writeLog({ event: 'advance_rejected', code: message.code, reason: message.reason });
    fs.writeFileSync(path.join(outDir, 'advance-result.json'), JSON.stringify({
      session_id: sessionId, command_id: commandId, completed: false,
      code: message.code ?? null, reason: message.reason ?? null,
    }, null, 2));
    finish(1);
  }
};
ws.onerror = () => fail('workspace WebSocket error');
ws.onclose = () => { clearTimeout(hardTimer); finish(advanced ? 0 : 1); };
