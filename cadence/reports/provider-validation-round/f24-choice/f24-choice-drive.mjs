#!/usr/bin/env node
/**
 * F-24 choice 卡主动触发验证驱动器。
 *
 * 场景：pi 跑 single_candidate work-item-plan（作者腿），需求描述内置强制
 * ask_user 裁定项 → pi 调 ask_user → 服务端广播 choice_request。
 *
 * 与 campaign 驱动器的差别：本驱动【拒绝应答 choice_request】——卡留给
 * 第二连接（浏览器 cockpit 页）应答，验证 F-24 重投面：
 *   attach 初帧补发挂起 choice（0484「引擎 pending、用户全程看不到」形态）。
 *
 * 事件全量落盘 ws.jsonl；choice pending 时写 marker 文件（浏览器侧轮询）。
 */
import fs from 'node:fs';
import path from 'node:path';

const REPO_ROOT = path.resolve('/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo');
const CAMPAIGN_DIR = path.join(REPO_ROOT, 'cadence/reports/workitem-coding-campaign');
const FIXTURES_DIR = path.join(CAMPAIGN_DIR, 'fixtures', 'minimal');
const BASE = (process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317').replace(/\/$/, '');
const WS_BASE = BASE.replace(/^http/, 'ws');
const PROJECT_ID = 'project_0001';
const REPOSITORY_ID = 'repository_0001';
const ARIA_ROOT = path.join(REPO_ROOT, '.aria');
const OUT_DIR = path.resolve(process.argv[2] ?? path.join(CAMPAIGN_DIR, '../provider-validation-round/f24-choice/run'));
const MARKER_FILE = process.env.F24_MARKER ?? '/tmp/f24-choice-pending.json';
const HARD_LIMIT_MS = Number(process.env.F24_HARD_LIMIT_MS ?? 15 * 60_000);

fs.mkdirSync(OUT_DIR, { recursive: true });
const log = fs.createWriteStream(path.join(OUT_DIR, 'ws.jsonl'), { flags: 'wx' });
const now = () => new Date().toISOString();
const writeLog = (entry) => log.write(`${JSON.stringify({ at: now(), ...entry })}\n`);
const json = (v) => JSON.stringify(v, null, 2);

// —— 需求描述：最小 Hello API 变体 + 强制 ask_user 用户裁定项 ——
const DESCRIPTION = `# 需求：naruto H5 游戏新增「最小 Greet API（问候文案待用户裁定）」

为 naruto H5 游戏（当前为空仓库骨架）提供一个最小后端问候接口，供联调冒烟使用。

## 需求

- REQ-001: GET /api/greet 返回 {"message":"<问候文案>"}。问候文案存在必须由用户现场裁定的歧义：候选为 hello / 你好 / こんにちは，三者都满足联调目的，选择权在用户。
- REQ-002: 仅用 Node 内置 node:http；不引入任何第三方依赖、不新增数据库/认证/外部服务/前端页面。

## 用户裁定流程（强制，先于一切产出）

- 在生成任何工作项内容之前，必须先调用 ask_user 工具向用户提问"问候文案选哪个：hello / 你好 / こんにちは"，等待用户答复；未获答复不得编写计划内容；收到答复后按用户裁定文案落实 REQ-001 与 AC-001（期望 body 中的文案与用户裁定一致）。

## 验收标准

- AC-001: GET /api/greet 返回 200 且 body 恰为 {"message":"<用户裁定的文案>"}。

约束：

- 单契约、单 source id（issue_workitem_0001#api）；不做任何其他拆分。
- 测试只用 Node 内置测试运行器（node --test），起服务请求 /api/greet 断言状态码与 body；只此一个测试。
`;

function rewriteFixture(value, issueId, timestamp) {
  if (Array.isArray(value)) return value.map((item) => rewriteFixture(item, issueId, timestamp));
  if (!value || typeof value !== 'object') return value;
  return Object.fromEntries(Object.entries(value).map(([key, nested]) => {
    if (key === 'issue_id') return [key, issueId];
    if (key.endsWith('_at')) return [key, timestamp];
    return [key, rewriteFixture(nested, issueId, timestamp)];
  }));
}

function writeJsonExclusive(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, json(value), { encoding: 'utf8', flag: 'wx' });
}

function seedFixtures(issueId) {
  const issueRoot = path.join(ARIA_ROOT, 'projects', PROJECT_ID, 'issues', issueId);
  if (!fs.existsSync(path.join(issueRoot, 'issue.json'))) {
    throw new Error(`issue store 不存在: ${issueRoot}`);
  }
  const timestamp = now();
  const destinations = {
    story_spec_0001: path.join(issueRoot, 'story-specs', 'story_spec_0001.json'),
    design_spec_0001: path.join(issueRoot, 'design-specs', 'design_spec_0001.json'),
    story_version_0001: path.join(issueRoot, 'versions', 'story_spec_0001', 'version_0001.json'),
    design_version_0001: path.join(issueRoot, 'versions', 'design_spec_0001', 'version_0001.json'),
  };
  for (const name of ['story_version_0001', 'design_version_0001', 'story_spec_0001', 'design_spec_0001']) {
    const fixture = JSON.parse(fs.readFileSync(path.join(FIXTURES_DIR, `${name}.json`), 'utf8'));
    writeJsonExclusive(destinations[name], rewriteFixture(fixture, issueId, timestamp));
  }
}

async function requestJson(url, options) {
  const response = await fetch(url, options);
  const text = await response.text();
  let body = {};
  try { body = text ? JSON.parse(text) : {}; } catch { body = { raw: text.slice(0, 1000) }; }
  if (!response.ok) throw new Error(`${url} -> ${response.status}: ${text.slice(0, 500)}`);
  return body;
}

const startedAt = Date.now();
const elapsed = () => Number(((Date.now() - startedAt) / 1000).toFixed(3));
const result = {
  scenario: 'f24-choice-card-pi-ask-user',
  base_url: BASE,
  started_at: now(),
  stageTimeline: [],
  choice: null,
  post_answer_events: [],
  provider_continued: null,
  terminal: null,
};

let ended = false;
let ws = null;
let startSent = false;
let currentStage = null;
let choiceSeenAt = null;
let answerEchoSeenAt = null;
let providerEventsSeen = 0;

function finish(exitCode, terminal) {
  if (ended) return;
  ended = true;
  if (terminal) result.terminal = terminal;
  result.provider_continued = result.post_answer_events.length > 0;
  result.elapsed_sec = elapsed();
  result.stage_durations_sec = Object.fromEntries(result.stageTimeline.map((e, i) => [
    `${i}:${e.stage}`,
    Math.round(((result.stageTimeline[i + 1]?.elapsedSec ?? result.elapsed_sec) - e.elapsedSec) * 1000) / 1000,
  ]));
  fs.writeFileSync(path.join(OUT_DIR, 'result.json'), json(result));
  try { ws?.close(); } catch { /* noop */ }
  log.end(() => process.exit(exitCode));
}

const hardTimer = setTimeout(() => finish(1, { kind: 'hard_timeout', at: now() }), HARD_LIMIT_MS);

const recordStage = (stage, source) => {
  if (typeof stage !== 'string') return;
  const last = result.stageTimeline.at(-1);
  if (last?.stage === stage) return;
  result.stageTimeline.push({ stage, elapsedSec: elapsed(), source });
};

function send(message) {
  writeLog({ direction: 'out', message });
  ws.send(JSON.stringify(message));
}

// continuation 判定：choice 应答（来自浏览器）之后的 provider 活动。
// 活动信号 = execution_event（pi 流增量/工具）| artifact_update | timeline 节点状态推进 | stage 推进。
function notePossibleContinuation(message) {
  if (!answerEchoSeenAt) return;
  const kinds = ['execution_event', 'artifact_update', 'timeline_node_updated', 'timeline_node_created'];
  if (kinds.includes(message.type)) {
    result.post_answer_events.push({
      type: message.type,
      at: now(),
      elapsedSec: elapsed(),
      brief: message.type === 'execution_event'
        ? String(message.event?.type ?? message.event?.kind ?? '')
        : (message.node_id ?? message.version ?? null),
    });
    if (result.post_answer_events.length > 200) result.post_answer_events.length = 200;
  }
}

async function main() {
  const issue = await requestJson(`${BASE}/api/projects/${PROJECT_ID}/issues`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      title: 'F-24 choice 卡验证（pi ask_user 裁定）',
      description: DESCRIPTION,
      repository_id: REPOSITORY_ID,
    }),
  });
  const issueId = issue.issue_id ?? issue.issue?.issue_id ?? issue.id;
  if (!issueId) throw new Error(`create issue 响应缺少 issue_id: ${json(issue)}`);
  result.issue_id = issueId;
  writeLog({ event: 'issue_created', issue_id: issueId });

  seedFixtures(issueId);
  writeLog({ event: 'fixtures_seeded' });

  const prepareOptions = {
    story_spec_ids: ['story_spec_0001'],
    design_spec_ids: ['design_spec_0001'],
    author_provider: 'pi',
    reviewer_provider: 'pi',
    review_rounds: 1,
    superpowers_enabled: true,
    openspec_enabled: true,
    run_policy: 'auto_if_valid',
    include_integration_tests: false,
    include_e2e_tests: false,
    force_frontend_backend_split: false,
    require_execution_plan_confirm: false,
  };
  const prepared = await requestJson(
    `${BASE}/api/projects/${PROJECT_ID}/issues/${issueId}/work-item-plans:prepare`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ title: 'F-24 choice 卡验证计划', ...prepareOptions }),
    },
  );
  const sessionId = prepared.workspace_session?.workspace_session_id
    ?? prepared.workspace_session?.session_id
    ?? prepared.workspace_session?.id
    ?? prepared.session_id;
  if (!sessionId) throw new Error(`prepare 响应缺少 workspace_session_id: ${json(prepared).slice(0, 800)}`);
  result.workspace_session_id = sessionId;
  result.plan_id = prepared.work_item_plan?.id ?? prepared.work_item_plan?.plan_id ?? prepared.plan_id ?? null;
  result.cockpit_url = `${BASE}/workbench/workspace/${sessionId}`;
  writeLog({ event: 'plan_prepared', session_id: sessionId });

  // 便于浏览器侧获取：session 元信息落 marker 前置文件（choice pending 时会覆盖为带 choice 的 marker）
  fs.writeFileSync('/tmp/f24-session.json', json({
    session_id: sessionId,
    issue_id: issueId,
    cockpit_url: result.cockpit_url,
    at: now(),
  }));

  ws = new WebSocket(`${WS_BASE}/api/workspace-sessions/${encodeURIComponent(sessionId)}/ws`);
  ws.onopen = () => {
    writeLog({ event: 'ws_open' });
    send({ type: 'hello', session_id: sessionId, last_seen_node_id: null });
  };
  ws.onmessage = (event) => {
    if (ended) return;
    let message;
    try { message = JSON.parse(event.data); } catch {
      writeLog({ direction: 'in', malformed_raw: String(event.data).slice(0, 500) });
      return;
    }
    writeLog({ direction: 'in', message_type: message.type, message });
    if (message.stage === 'prepare_context' && !startSent) {
      // 初始 session_state 即携带 stage=prepare_context（无独立 stage_change 帧）；
      // 与 campaign 驱动器同语义：stage 信号不分消息来源。
      startSent = true;
      send({
        type: 'start_generation',
        provider_config: {
          author: 'pi',
          reviewer: 'pi',
          review_rounds: 1,
          permission_modes: { author: 'auto', reviewer: 'auto' },
        },
        reviewer_enabled: true,
      });
      return;
    }
    if (message.type === 'session_state') {
      recordStage(message.stage ?? message.session?.stage, 'session_state');
      if (message.session_status && message.session_status !== 'running') {
        // completed / stopped 等终态
      }
    }
    if (message.stage) recordStage(message.stage, message.type);
    notePossibleContinuation(message);

    if (message.type === 'choice_request') {
      choiceSeenAt = Date.now();
      result.choice = {
        id: message.id,
        prompt: message.prompt ?? null,
        source: message.source ?? null,
        options: (message.options ?? []).map((o) => ({ id: o.id, label: o.label })),
        allow_multiple: message.allow_multiple ?? false,
        allow_free_text: message.allow_free_text ?? false,
        seenAt: now(),
        seenElapsedSec: elapsed(),
      };
      // marker：浏览器侧轮询此文件后打开 cockpit 页
      fs.writeFileSync(MARKER_FILE, json({ ...result.choice, session_id: sessionId, cockpit_url: result.cockpit_url }));
      writeLog({ event: 'choice_pending_not_answered_by_driver', choice_id: message.id });
      // 显式不发送 choice_response —— 应答必须来自第二连接（浏览器）
      return;
    }
    // choice_response 的服务端回执/广播（不同实现可能透出 choice_resolved 或 chat 回执；先宽松捕获）
    if (
      (message.type === 'choice_response' || message.type === 'choice_resolved')
      && result.choice && message.id === result.choice.id
    ) {
      answerEchoSeenAt = Date.now();
      result.choice.answerEchoAt = now();
      result.choice.answerEchoElapsedSec = elapsed();
      writeLog({ event: 'choice_answered_via_second_connection', choice_id: message.id });
    }
    // execution_event 在 choice 应答后恢复 = provider 继续（兜底信号，不依赖显式回执帧）
    if (message.type === 'execution_event' && result.choice && !answerEchoSeenAt && choiceSeenAt) {
      // choice 挂起期间通常无流事件；一旦出现流事件即视为恢复（保守：要求距 choice ≥1s）
      if (Date.now() - choiceSeenAt > 1000) {
        answerEchoSeenAt = Date.now();
        result.choice.streamResumedAt = now();
        result.choice.streamResumedElapsedSec = elapsed();
        writeLog({ event: 'provider_stream_resumed_after_choice' });
      }
    }
    if (message.type === 'execution_event') providerEventsSeen += 1;

    if (message.type === 'stage_change' && message.stage === 'prepare_context' && !startSent) {
      startSent = true;
      send({
        type: 'start_generation',
        provider_config: {
          author: 'pi',
          reviewer: 'pi',
          review_rounds: 1,
          permission_modes: { author: 'auto', reviewer: 'auto' },
        },
        reviewer_enabled: true,
      });
      return;
    }
    if (message.type === 'stage_change' && message.stage === 'completed') {
      finish(0, { kind: 'completed', at: now(), elapsedSec: elapsed() });
    }
    if (message.type === 'session_state' && ['completed', 'stopped', 'failed'].includes(message.session_status)) {
      finish(0, { kind: message.session_status, at: now(), elapsedSec: elapsed(), stage: currentStage });
    }
  };
  ws.onerror = (error) => {
    writeLog({ event: 'ws_error', error: String(error?.message ?? error) });
    if (!ended) finish(1, { kind: 'ws_error', at: now() });
  };
  ws.onclose = () => {
    writeLog({ event: 'ws_close' });
    if (!ended) finish(1, { kind: 'ws_closed_unexpected', at: now() });
  };
}

main().catch((error) => {
  writeLog({ event: 'fatal', error: String(error?.stack ?? error) });
  finish(1, { kind: 'setup_error', error: String(error?.message ?? error) });
});
