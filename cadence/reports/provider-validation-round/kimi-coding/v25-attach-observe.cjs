// v25 续跑验证：attach coding attempt 0556a410，观察并记录会话状态
const WS_DIR = '/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/web/node_modules/.pnpm/ws@8.20.0/node_modules';
const { WebSocket } = require(WS_DIR + '/ws');
const fs = require('fs');

const ATTEMPT = 'coding_attempt_0556a410c0de429db96c9550c0b80fa2';
const URL = `ws://127.0.0.1:4317/ws/coding-attempts/${ATTEMPT}`;
const OUT = '/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/reports/provider-validation-round/kimi-coding/coding-kimi_code-coding_attempt_0556a410c0de429db96c9550c0b80fa2/v25-resume-ws.jsonl';

const log = (obj) => fs.appendFileSync(OUT, JSON.stringify({ at: new Date().toISOString(), ...obj }) + '\n');
const seen = { in: 0, out: 0 };

const ws = new WebSocket(URL);
const t0 = Date.now();

ws.on('open', () => {
  log({ event: 'ws_open', url: URL });
  ws.send(JSON.stringify({ type: 'coding_hello', attempt_id: ATTEMPT, last_seen_node_id: null }));
  log({ direction: 'out', message: { type: 'coding_hello', attempt_id: ATTEMPT, last_seen_node_id: null } });
});
ws.on('message', (data) => {
  let m;
  try { m = JSON.parse(data.toString()); } catch { m = { parse_error: data.toString().slice(0, 200) }; }
  const t = m.type || '?';
  seen.in++;
  // 全量记录控制帧；stream chunk 只记类型+计数（体积控制）
  if (t === 'coding_stream_chunk') {
    log({ direction: 'in', message: { type: t }, note: 'chunk-elided' });
  } else {
    log({ direction: 'in', message: m });
  }
  if (t === 'coding_session_state') {
    console.log('[session_state] status=' + m.status + ' stage=' + m.stage +
      ' units=' + JSON.stringify((m.units || []).map(u => ({ id: u.unit_id, s: u.status }))) +
      ' rework=' + m.rework_count + '/' + m.max_auto_rework +
      ' manual_recovery_reason=' + (m.manual_recovery_reason || 'null') +
      ' head=' + (m.head_commit || 'null'));
  } else if (t === 'coding_gate_required') {
    console.log('[gate_required] ' + JSON.stringify({ gate_id: m.gate && m.gate.gate_id, kind: m.gate && m.gate.kind, title: m.gate && m.gate.title }));
  } else if (t === 'coding_protocol_error') {
    console.log('[protocol_error] ' + JSON.stringify(m).slice(0, 600));
  } else if (t !== 'coding_stream_chunk' && t !== 'coding_execution_event') {
    console.log('[in:' + t + '] ' + JSON.stringify(m).slice(0, 240));
  }
});
ws.on('close', (code, reason) => {
  log({ event: 'ws_close', code, reason: reason.toString() });
  console.log('WS CLOSED code=' + code + ' reason=' + reason.toString());
  report();
});
ws.on('error', (err) => {
  log({ event: 'ws_error', error: String(err && err.message || err) });
  console.log('WS ERROR ' + (err && err.message));
});

function report() {
  log({ event: 'summary', seen, elapsedMs: Date.now() - t0 });
  console.log('SUMMARY ' + JSON.stringify(seen));
}

// 观察 90 秒：覆盖 attach→快照→(若发生) runner 重启判定窗口
setTimeout(() => {
  try { ws.close(1000, 'observation-window-elapsed'); } catch {}
  setTimeout(() => { try { ws.terminate(); } catch {} report(); process.exit(0); }, 2000);
}, 90000);
