// wave2-2c F-12 复测驱动：plan 会话 start_generation → prompt 落盘 → provider 子进程拉起 精确计时。
// 用法: node f12_plan_timing.mjs <rep> <provider=pi>
// 证据: 同目录 f12-rep<N>-evidence.json / f12-rep<N>-ws.jsonl
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..', '..', '..', '..');
const DESCRIPTION_FILE = path.join(REPO_ROOT, 'cadence', 'reports', 'design-weak-model-campaign', 'corpus', '08-minimal-hello-api.md');
const ARIA_ROOT = path.join(REPO_ROOT, '.aria');
const BASE = 'http://127.0.0.1:4317';
const WS_BASE = BASE.replace(/^http/, 'ws');
const PROJECT_ID = 'project_0001';
const REPOSITORY_ID = 'repository_0001';
const SERVER_PID = '2698026';
const FIXTURES_DIR = path.join(REPO_ROOT, 'cadence', 'reports', 'workitem-coding-campaign', 'fixtures', 'minimal');
const PROVIDER = process.argv[3] ?? 'pi';
const REP = process.argv[2] ?? '1';
const SPAWN_WAIT_MS = 10 * 60_000; // 10min 无子进程=超时登记

const logLines = [];
const stageTimeline = [];
let t0Wall = null;
let t0Mono = null;
const now = () => new Date().toISOString();
const mono = () => Number(process.hrtime.bigint() / 1_000n) / 1_000; // ms float

function wsLog(entry) {
  const line = JSON.stringify({ t: now(), ...entry });
  logLines.push(line);
  fs.appendFileSync(path.join(HERE, `f12-rep${REP}-ws.jsonl`), line + '\n');
}

function psChildren() {
  try {
    const out = execFileSync('ps', ['--ppid', SERVER_PID, '-o', 'pid=,lstart=,args='], { encoding: 'utf8', timeout: 5_000 });
    return out.split('\n').map((l) => l.trim()).filter(Boolean);
  } catch {
    return [];
  }
}

function seedFixtures(issueId) {
  const issueRoot = path.join(ARIA_ROOT, 'projects', PROJECT_ID, 'issues', issueId);
  if (!fs.existsSync(path.join(issueRoot, 'issue.json'))) {
    throw new Error(`issue store 不存在: ${issueRoot}`);
  }
  const timestamp = now();
  const rewrite = (value) => {
    if (Array.isArray(value)) return value.map(rewrite);
    if (!value || typeof value !== 'object') return value;
    return Object.fromEntries(Object.entries(value).map(([key, nested]) => {
      if (key === 'issue_id') return [key, issueId];
      if (key.endsWith('_at')) return [key, timestamp];
      return [key, rewrite(nested)];
    }));
  };
  const destinations = {
    story_spec_0001: path.join(issueRoot, 'story-specs', 'story_spec_0001.json'),
    design_spec_0001: path.join(issueRoot, 'design-specs', 'design_spec_0001.json'),
    story_version_0001: path.join(issueRoot, 'versions', 'story_spec_0001', 'version_0001.json'),
    design_version_0001: path.join(issueRoot, 'versions', 'design_spec_0001', 'version_0001.json'),
  };
  for (const name of ['story_version_0001', 'design_version_0001', 'story_spec_0001', 'design_spec_0001']) {
    const fixture = JSON.parse(fs.readFileSync(path.join(FIXTURES_DIR, `${name}.json`), 'utf8'));
    const dest = destinations[name];
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.writeFileSync(dest, JSON.stringify(rewrite(fixture), null, 2), { flag: 'wx' });
  }
  return { issueRoot, seededAt: timestamp };
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

function promptDetailDir(sessionId, issueId) {
  return path.join(ARIA_ROOT, 'projects', PROJECT_ID, 'issues', issueId, 'workspace-timelines', sessionId, 'timeline_node_details');
}

function findPromptOnDisk(dir) {
  try {
    for (const f of fs.readdirSync(dir)) {
      if (!f.endsWith('.json')) continue;
      try {
        const d = JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8'));
        if (typeof d.prompt === 'string' && d.prompt.length > 0) {
          return { file: f, stat: fs.statSync(path.join(dir, f)) };
        }
      } catch { /* 半写状态忽略 */ }
    }
  } catch { /* 目录尚不存在 */ }
  return null;
}

const result = {
  rep: REP,
  provider: PROVIDER,
  server_pid: SERVER_PID,
  started_at: now(),
  issue_id: null,
  plan_id: null,
  workspace_session_id: null,
  baseline_children: [],
  t0_click: null,
  t1_prompt_disk: null,
  t1_prompt_file: null,
  t1_prompt_mtime: null,
  t2_spawn_detected: null,
  t2_spawn_lstart: null,
  t2_spawn_cmd: null,
  delta_click_to_prompt_ms: null,
  delta_prompt_to_spawn_ms: null,
  delta_click_to_spawn_ms: null,
  ws_prompt_event: null,
  stage_timeline: stageTimeline,
  abort_sent_at: null,
  abort_final_stage: null,
  session_status_after_abort: null,
  outcome: null,
};

async function main() {
  result.baseline_children = psChildren();
  const description = fs.readFileSync(DESCRIPTION_FILE, 'utf8');

  // 1. 建 issue
  const issue = await requestJson(`${BASE}/api/projects/${PROJECT_ID}/issues`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      title: `wave2-2c F-12 timing rep${REP} ${PROVIDER}`,
      description,
      repository_id: REPOSITORY_ID,
    }),
    label: 'create issue',
  });
  result.issue_id = issue.issue_id ?? issue.issue?.issue_id ?? issue.id;
  if (!result.issue_id) throw new Error(`create issue 响应缺 issue_id: ${JSON.stringify(issue).slice(0, 300)}`);
  wsLog({ event: 'issue_created', issue_id: result.issue_id });

  // 2. seed minimal fixtures
  seedFixtures(result.issue_id);
  wsLog({ event: 'fixtures_seeded' });

  // 3. prepare work-item-plan（campaign 同款选项）
  const prepared = await requestJson(`${BASE}/api/projects/${PROJECT_ID}/issues/${result.issue_id}/work-item-plans:prepare`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      title: `wave2-2c F-12 timing rep${REP}`,
      story_spec_ids: ['story_spec_0001'],
      design_spec_ids: ['design_spec_0001'],
      author_provider: PROVIDER,
      reviewer_provider: PROVIDER,
      review_rounds: 1,
      superpowers_enabled: true,
      openspec_enabled: true,
      run_policy: 'auto_if_valid',
      include_integration_tests: false,
      include_e2e_tests: false,
      force_frontend_backend_split: false,
      require_execution_plan_confirm: false,
    }),
    label: 'prepare plan',
  });
  result.plan_id = prepared.work_item_plan?.id ?? prepared.work_item_plan?.plan_id ?? prepared.plan_id;
  result.workspace_session_id = prepared.workspace_session?.workspace_session_id ?? prepared.workspace_session?.session_id ?? prepared.workspace_session?.id ?? prepared.session_id;
  if (!result.plan_id || !result.workspace_session_id) {
    throw new Error(`prepare 响应缺 plan_id/session_id: ${JSON.stringify(prepared).slice(0, 400)}`);
  }
  result.prepare_flow_kind = prepared.workspace_session?.flow_kind ?? null;
  wsLog({ event: 'plan_prepared', plan_id: result.plan_id, session_id: result.workspace_session_id, flow_kind: result.prepare_flow_kind });

  const detailDir = promptDetailDir(result.workspace_session_id, result.issue_id);

  // 4. WS 连接
  const ws = new WebSocket(`${WS_BASE}/api/workspace-sessions/${encodeURIComponent(result.workspace_session_id)}/ws`);
  let startSent = false;
  let abortSent = false;
  let promptFound = false;
  let spawnFound = false;
  let spawnDetectionLagMs = null;
  let done = false;
  const finish = (outcome) => {
    if (done) return;
    done = true;
    result.outcome = outcome;
    result.stage_timeline = stageTimeline;
    try { ws.close(); } catch { /* ignore */ }
  };

  ws.onmessage = (event) => {
    let msg = null;
    try { msg = JSON.parse(event.data); } catch { return; }
    const type = msg?.type;
    if (type === 'session_state') {
      const stage = msg.stage;
      if (stage && (stageTimeline.length === 0 || stageTimeline[stageTimeline.length - 1].stage !== stage)) {
        stageTimeline.push({ stage, at: now(), mono_ms: mono() });
      }
      if (stage === 'prepare_context' && !startSent) {
        startSent = true;
        // —— 点击开始生成 ——
        t0Wall = now();
        t0Mono = mono();
        result.t0_click = t0Wall;
        wsLog({ event: 'start_generation_sent', at: t0Wall, provider: PROVIDER });
        ws.send(JSON.stringify({
          type: 'start_generation',
          provider_config: {
            author: PROVIDER,
            reviewer: PROVIDER,
            review_rounds: 1,
            permission_modes: { author: 'auto', reviewer: 'auto' },
          },
          reviewer_enabled: true,
        }));
        return;
      }
    }
    if (type === 'stage_change') {
      const stage = msg.stage ?? msg.to;
      if (stage) wsLog({ event: 'stage_change', stage });
    }
    if (type === 'execution_event' || type === 'timeline_event' || (type && String(type).includes('prompt'))) {
      wsLog({ event: 'ws_message', type, title: msg.title ?? null, node_id: msg.node_id ?? null });
      if (!result.ws_prompt_event && (String(type).includes('prompt') || String(msg.title ?? '').includes('Prompt'))) {
        result.ws_prompt_event = { type, at: now(), delta_from_click_ms: t0Mono ? mono() - t0Mono : null };
      }
    }
    if (type === 'error' || type === 'protocol_error') {
      wsLog({ event: 'server_error', type, message: msg.message ?? null, code: msg.code ?? null });
    }
  };
  ws.onopen = () => {
    wsLog({ event: 'ws_open' });
    ws.send(JSON.stringify({ type: 'hello', session_id: result.workspace_session_id, last_seen_node_id: null }));
  };
  ws.onerror = (e) => wsLog({ event: 'ws_error', message: e?.message ?? null });
  ws.onclose = (e) => wsLog({ event: 'ws_close', code: e?.code, reason: e?.reason });

  // 5. 轮询:prompt 落盘 + provider 子进程
  const pollStart = mono();
  const pollInterval = setInterval(() => {
    if (t0Mono === null) return;
    if (!promptFound) {
      const hit = findPromptOnDisk(detailDir);
      if (hit) {
        promptFound = true;
        result.t1_prompt_disk = now();
        result.t1_prompt_file = hit.file;
        result.t1_prompt_mtime = hit.stat.mtime.toISOString();
        result.delta_click_to_prompt_ms = Math.round(mono() - t0Mono);
        wsLog({ event: 'prompt_on_disk', file: hit.file, delta_from_click_ms: result.delta_click_to_prompt_ms });
      }
    }
    if (!spawnFound) {
      const newChildren = psChildren().filter((line) => !result.baseline_children.includes(line));
      // 版本探测进程（pi --version）单独标记，不算真实 provider 拉起
      if (!result.t_probe) {
        const probe = newChildren.find((line) => /--version/.test(line) && /(\/| )pi( |$)/.test(line));
        if (probe) {
          result.t_probe = { at: now(), cmd: probe, delta_from_click_ms: Math.round(mono() - t0Mono) };
          wsLog({ event: 'pi_version_probe_detected', cmd: probe, delta_from_click_ms: result.t_probe.delta_from_click_ms });
        }
      }
      // 真实 provider 会话进程：pi --mode rpc（含 -e 扩展），排除 --version 探测
      const providerChildren = newChildren.filter((line) => /--mode\s+rpc/.test(line));
      if (providerChildren.length > 0) {
        const children = providerChildren;
        spawnFound = true;
        spawnDetectionLagMs = mono();
        const first = children[0];
        const pid = first.split(/\s+/)[0];
        result.t2_spawn_detected = now();
        result.t2_spawn_cmd = first;
        result.t2_all_new_children = children;
        try {
          const lstart = execFileSync('ps', ['-p', pid, '-o', 'lstart='], { encoding: 'utf8' }).trim();
          result.t2_spawn_lstart = lstart;
        } catch { /* 已退出则空 */ }
        result.t2_all_new_children = newChildren;
        result.delta_click_to_spawn_ms = Math.round(mono() - t0Mono);
        result.delta_prompt_to_spawn_ms = result.t1_prompt_disk ? Math.round(mono() - t0Mono - result.delta_click_to_prompt_ms) : null;
        wsLog({ event: 'provider_subprocess_spawn_detected', pid, cmd: first, delta_from_click_ms: result.delta_click_to_spawn_ms });

        // 稳定 1.5s 后 Abort，节约 token（测量已完成）
        setTimeout(() => {
          if (abortSent || done) return;
          abortSent = true;
          result.abort_sent_at = now();
          wsLog({ event: 'abort_sent' });
          try { ws.send(JSON.stringify({ type: 'abort' })); } catch { /* ignore */ }
          setTimeout(() => {
            try {
              const sess = JSON.parse(fs.readFileSync(path.join(ARIA_ROOT, 'projects', PROJECT_ID, 'issues', result.issue_id, 'workspace-sessions', `${result.workspace_session_id}.json`), 'utf8'));
              result.session_status_after_abort = sess.status;
              result.abort_final_stage = sess.stage ?? null;
            } catch { /* ignore */ }
            wsLog({ event: 'post_abort_session_status', status: result.session_status_after_abort });
            finish('spawn_measured_and_aborted');
          }, 3000);
        }, 1500);
      }
    }
    if (mono() - pollStart > SPAWN_WAIT_MS && !spawnFound) {
      wsLog({ event: 'spawn_wait_timeout', waited_ms: SPAWN_WAIT_MS });
      result.outcome_timeout = true;
      clearInterval(pollInterval);
      finish('spawn_timeout_10min');
    }
  }, 150);

  // 收尾兜底
  setTimeout(() => {
    clearInterval(pollInterval);
    finish(spawnFound ? 'spawn_measured_and_aborted' : 'incomplete_hard_timeout');
  }, SPAWN_WAIT_MS + 60_000);

  const waitDone = () => new Promise((resolve) => {
    const check = setInterval(() => {
      if (done) { clearInterval(check); resolve(); }
    }, 200);
  });
  await waitDone();
}

try {
  await main();
} catch (error) {
  result.fatal = String(error?.stack ?? error);
  result.outcome = 'driver_error';
}
result.completed_at = now();
fs.writeFileSync(path.join(HERE, `f12-rep${REP}-evidence.json`), JSON.stringify(result, null, 2));
console.log(JSON.stringify({
  rep: result.rep,
  outcome: result.outcome,
  delta_click_to_prompt_ms: result.delta_click_to_prompt_ms,
  delta_prompt_to_spawn_ms: result.delta_prompt_to_spawn_ms,
  delta_click_to_spawn_ms: result.delta_click_to_spawn_ms,
  t2_spawn_cmd: result.t2_spawn_cmd,
  fatal: result.fatal ?? null,
}, null, 2));
process.exit(0);
