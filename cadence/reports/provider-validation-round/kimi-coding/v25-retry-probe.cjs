// wire 实证：awaiting_manual_recovery 状态下 retry 动作可用性探测
// 依据 socket.rs message_allowed：AwaitingManualRecovery 仅接受 AbortAttempt；
// 本脚本发 gate_response(retry_coding) 与 start_coding，记录服务端拒绝帧。不 abort。
const WS_DIR = '/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/web/node_modules/.pnpm/ws@8.20.0/node_modules';
const { WebSocket } = require(WS_DIR + '/ws');
const fs = require('fs');

const ATTEMPT = 'coding_attempt_0556a410c0de429db96c9550c0b80fa2';
const URL = `ws://127.0.0.1:4317/ws/coding-attempts/${ATTEMPT}`;
const OUT = '/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/reports/provider-validation-round/kimi-coding/coding-kimi_code-coding_attempt_0556a410c0de429db96c9550c0b80fa2/v25-retry-probe-ws.jsonl';
const log = (obj) => fs.appendFileSync(OUT, JSON.stringify({ at: new Date().toISOString(), ...obj }) + '\n');

const ws = new WebSocket(URL);
ws.on('open', () => {
  log({ event: 'ws_open' });
  ws.send(JSON.stringify({ type: 'coding_hello', attempt_id: ATTEMPT, last_seen_node_id: null }));
  log({ direction: 'out', message: { type: 'coding_hello' } });
});
let step = 0;
ws.on('message', (data) => {
  let m; try { m = JSON.parse(data.toString()); } catch { m = { raw: data.toString().slice(0, 200) }; }
  const t = m.type || '?';
  log({ direction: 'in', message: t === 'coding_stream_chunk' ? { type: t } : m });
  console.log('[in:' + t + '] ' + JSON.stringify(m).slice(0, 300));

  if (t === 'coding_session_state') {
    setTimeout(() => {
      // 探测 1：Main 指示的人工恢复 retry gate_response（最近 resolved 的 blocked gate）
      const msg1 = { type: 'gate_response', gate_id: 'coding_blocked_gate_0002', action_id: 'retry_coding', extra_context: 'v25 retry probe per UI semantics' };
      ws.send(JSON.stringify(msg1)); log({ direction: 'out', message: msg1 });
    }, 1000);
    setTimeout(() => {
      // 探测 2：start_coding（Coding 态重开）
      const msg2 = { type: 'start_coding' };
      ws.send(JSON.stringify(msg2)); log({ direction: 'out', message: msg2 });
    }, 4000);
    setTimeout(() => {
      // 探测 3：stage_gate_confirm（stage gate 0010 补确认）
      const msg3 = { type: 'stage_gate_confirm', stage: 'coding' };
      ws.send(JSON.stringify(msg3)); log({ direction: 'out', message: msg3 });
    }, 7000);
  }
});
ws.on('close', (c, r) => { log({ event: 'ws_close', code: c, reason: r.toString() }); console.log('CLOSED', c); process.exit(0); });
ws.on('error', (e) => { log({ event: 'ws_error', error: String(e) }); console.log('ERR', e.message); });
setTimeout(() => { try { ws.close(1000, 'probe-done'); } catch {}; setTimeout(() => process.exit(0), 1500); }, 15000);
