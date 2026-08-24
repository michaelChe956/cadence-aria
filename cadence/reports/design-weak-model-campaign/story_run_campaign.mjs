// Story campaign single-sample driver（/tmp/gate-campaign.mjs 的 campaign 目录副本，
// 增加 usage 采集：从 WS execution_event（kind=usage）提取 per-role token 用量写入
// result.usage_by_role；原 /tmp 稿不改动）。
// Usage: node story_run_campaign.mjs <provider> <shapeId:01..05> <rep:1|2> <outRoot>
// Per-phase limits: author 300s, reviewer 300s, hard cap 700s. Choice -> first option.
import fs from 'node:fs';
import path from 'node:path';

const [, , provider, shapeId, rep, outRoot = '/tmp/gate-campaign'] = process.argv;
const BASE = 'http://127.0.0.1:4317';
const WS_BASE = 'ws://127.0.0.1:4317';
const CORPUS_DIR = '/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/reports/story-weak-model-campaign/corpus';
const AUTHOR_LIMIT = 600;
const REVIEWER_LIMIT = 300;
const HARD_LIMIT = 1000;

const shapeFile = fs.readdirSync(CORPUS_DIR).find((f) => f.startsWith(shapeId + '-'));
if (!shapeFile) { console.error('no corpus file for shape', shapeId); process.exit(2); }
const corpus = fs.readFileSync(path.join(CORPUS_DIR, shapeFile), 'utf8');

const outDir = path.join(outRoot, provider, `${shapeId}-rep${rep}`);
fs.mkdirSync(outDir, { recursive: true });
const ts = () => new Date().toISOString();
const log = fs.createWriteStream(path.join(outDir, 'ws.jsonl'));
const wlog = (o) => log.write(JSON.stringify({ t: Date.now(), ...o }) + '\n');
const note = (m) => { console.log(`[${provider}/${shapeId}/r${rep}] ${m}`); wlog({ note: m }); };

// §usage-collection：从 WS execution_event（kind=usage）提取 per-role token 用量。
// output 为 UsageReportData JSON（role/input/output/cache_read/cache_creation，缺失为 null）。
function collectUsageByRole(value, byRole) {
  if (!value || typeof value !== 'object') return;
  if (Array.isArray(value)) {
    for (const item of value) collectUsageByRole(item, byRole);
    return;
  }
  if (value.kind === 'usage' && typeof value.output === 'string') {
    try {
      const report = JSON.parse(value.output);
      const role = String(report.role ?? 'unknown');
      byRole[role] = {
        input_tokens: report.input_tokens ?? null,
        output_tokens: report.output_tokens ?? null,
        cache_read_tokens: report.cache_read_tokens ?? null,
        cache_creation_tokens: report.cache_creation_tokens ?? null,
      };
    } catch {
      // malformed usage output——best-effort，忽略
    }
    return;
  }
  for (const nested of Object.values(value)) collectUsageByRole(nested, byRole);
}

const usageByRole = {};

const result = {
  provider, shape_id: shapeId, shape_file: shapeFile, repetition: rep,
  case_id: { shape_id: shapeId, repetition_id: Number(rep) },
  startedAt: ts(), issueId: null, sessionId: null, storySpecId: null,
  firstArtifactSec: null, authorConfirmSec: null, reviewCompleteSec: null, finalizeSec: null,
  finished: false, authorConfirmed: false, choices: [], permissionApprovals: 0,
  nodes: [], reviewVerdicts: [], error: null, errorCode: null, failureClass: null,
  aborted: false,
};

const issueRes = await fetch(`${BASE}/api/projects/project_0001/issues`, {
  method: 'POST', headers: { 'content-type': 'application/json' },
  body: JSON.stringify({
    title: `Gate ${provider} ${shapeId} r${rep} story sample`,
    description: corpus, repository_id: 'repository_0001',
  }),
});
if (!issueRes.ok) { result.failureClass = 'provider_error'; result.error = `issue create ${issueRes.status}`; fs.writeFileSync(path.join(outDir, 'result.json'), JSON.stringify(result)); process.exit(1); }
const issue = await issueRes.json();
result.issueId = issue.issue?.issue_id ?? issue.issue_id;
note(`issue ${result.issueId}`);

const genRes = await fetch(`${BASE}/api/projects/project_0001/issues/${result.issueId}/story-specs:generate`, {
  method: 'POST', headers: { 'content-type': 'application/json' },
  body: JSON.stringify({
    title: `Gate ${provider} ${shapeId} r${rep} Story Spec`,
    author_provider: provider, reviewer_provider: provider,
    review_rounds: 1, superpowers_enabled: true, openspec_enabled: true,
  }),
});
if (!genRes.ok) { result.failureClass = 'provider_error'; result.error = `story-specs generate ${genRes.status}`; fs.writeFileSync(path.join(outDir, 'result.json'), JSON.stringify(result)); process.exit(1); }
const gen = await genRes.json();
result.sessionId = gen.workspace_session?.workspace_session_id ?? gen.workspace_session?.session_id;
result.storySpecId = gen.story_spec?.story_spec_id ?? gen.story_spec_id ?? null;
note(`session ${result.sessionId}`);

const ws = new WebSocket(`${WS_BASE}/api/workspace-sessions/${result.sessionId}/ws`);
const t0 = Date.now();
let sentStart = false, reviewDone = false, finalizeSent = false, authorStart = null, reviewStart = null;
const el = () => Number(((Date.now() - t0) / 1000).toFixed(1));
const send = (o) => { wlog({ out: o }); ws.send(JSON.stringify(o)); };

let phaseTimers = [];
function clearTimers() { phaseTimers.forEach(clearTimeout); phaseTimers = []; }
function armPhase(label, ms) {
  phaseTimers.push(setTimeout(() => {
    result.failureClass = label;
    note(`PHASE TIMEOUT: ${label} after ${ms / 1000}s`);
    finish(true);
  }, ms));
}
let finishing = false;
function finish(aborted) {
  if (finishing) return; finishing = true;
  clearTimers();
  if (aborted) result.aborted = true;
  if (!result.failureClass && result.finished) result.failureClass = 'completed';
  else if (!result.failureClass) result.failureClass = result.error ? 'provider_error' : 'incomplete';
  result.finishedAt = ts();
  result.elapsedSec = el();
  result.usage_by_role = Object.keys(usageByRole).length
    ? usageByRole
    : { usage_unavailable: true };
  try { ws.close(); } catch {}
  // artifact markdown dump
  if (result.artifactMarkdown) fs.writeFileSync(path.join(outDir, 'artifact.md'), result.artifactMarkdown);
  delete result.artifactMarkdown;
  fs.writeFileSync(path.join(outDir, 'result.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
  setTimeout(() => process.exit(0), 200);
}

armPhase('hard_timeout', HARD_LIMIT * 1000);

ws.onopen = () => { note('ws open'); send({ type: 'hello', session_id: result.sessionId, last_seen_node_id: null }); };
ws.onmessage = (ev) => {
  let msg; try { msg = JSON.parse(ev.data); } catch { return; }
  collectUsageByRole(msg, usageByRole);
  const type = msg.type;
  if (msg.event?.event_id || type === 'execution_event') {
    const e = msg.event ?? msg;
    wlog({ exec: { kind: e.kind, status: e.status, title: e.title, exit: e.exit_code, out: (e.output ?? '').slice(0, 300) } });
    if (e.status === 'failed' && /terminal capability|isError/.test(e.output ?? '')) {
      result.toolFailures = (result.toolFailures ?? 0) + 1;
    }
  } else wlog({ in: type === 'session_state' ? { type, stage: msg.stage } : msg });

  const nodesArr = msg.timeline_nodes ?? (msg.type === 'timeline_node_created' ? [msg.node] : null);
  if (nodesArr) for (const n of nodesArr) trackNode(n);
  if (msg.type === 'timeline_node_created') trackNode(msg.node);
  if (msg.type === 'timeline_node_updated') {
    const i = result.nodes.findIndex((x) => x.node_id === msg.node_id);
    if (i >= 0) { result.nodes[i].status = msg.status ?? result.nodes[i].status; result.nodes[i].summary = msg.summary ?? result.nodes[i].summary; }
  }

  switch (type) {
    case 'session_state': case 'stage_change': {
      const stage = msg.stage;
      if (!sentStart && stage === 'prepare_context') {
        sentStart = true; authorStart = Date.now();
        note('sending start_generation');
        send({
          type: 'start_generation',
          provider_config: { author: provider, reviewer: provider, review_rounds: 1, permission_modes: { author: 'auto', reviewer: 'auto' } },
          reviewer_enabled: true,
        });
        armPhase('author_timeout', AUTHOR_LIMIT * 1000);
      } else if (stage === 'author_confirm' && sentStart) {
        if (!result.authorConfirmed) {
          result.authorConfirmed = true;
          result.authorConfirmSec = el();
          if (result.firstArtifactSec === null) result.firstArtifactSec = el();
          note('author_confirm -> accept_with_review');
          reviewStart = Date.now();
          clearTimers(); armPhase('hard_timeout', HARD_LIMIT * 1000); armPhase('reviewer_timeout', REVIEWER_LIMIT * 1000 + 60_000);
          send({ type: 'author_decision', decision: 'accept_with_review' });
        } else if (reviewDone && !finalizeSent) {
          finalizeSent = true; result.finalizeSec = el();
          clearTimers(); armPhase('finalize_timeout', 60_000);
          note('author_confirm after review -> accept_finalize');
          send({ type: 'author_decision', decision: 'accept_finalize' });
        }
      }
      if (['confirmed', 'finalized', 'story_confirmed', 'completed'].includes(stage)) {
        result.finished = true; note(`terminal stage ${stage}`); setTimeout(() => finish(false), 800);
      }
      break;
    }
    case 'artifact_update': {
      if (result.firstArtifactSec === null) { result.firstArtifactSec = el(); note(`first artifact v${msg.version} @${el()}s`); }
      result.artifactVersion = msg.version;
      if (msg.markdown) result.artifactMarkdown = msg.markdown;
      break;
    }
    case 'choice_request': {
      const opts = msg.options ?? [];
      const questions = msg.questions ?? [];
      const answers = questions.map((q) => ({ question_id: q.question_id ?? q.id, selected_option_ids: (q.options ?? []).length ? [q.options[0].id] : [], free_text: null }));
      const top = opts.length ? [opts[0].id] : answers.flatMap((a) => a.selected_option_ids);
      result.choices.push({ el: el(), prompt: (msg.prompt ?? '').slice(0, 120), picked: top });
      note(`choice -> ${JSON.stringify(top)}`);
      send({ type: 'choice_response', id: msg.id, selected_option_ids: top, free_text: null, answers });
      break;
    }
    case 'permission_request':
      result.permissionApprovals++; send({ type: 'permission_response', id: msg.id, approved: true, reason: null }); break;
    case 'review_complete':
      reviewDone = true;
      result.reviewCompleteSec = el();
      result.reviewVerdicts.push({ el: el(), verdict: msg.verdict });
      note(`review_complete verdict=${msg.verdict}`);
      // after review, expect author_confirm (revision or finalize); give it time under hard cap
      clearTimers(); armPhase('finalize_timeout', 90_000);
      break;
    case 'error':
      result.error = msg.message; result.errorCode = msg.code ?? 'error'; note(`ERROR ${msg.message}`); break;
    case 'protocol_error':
      result.error = `${msg.code}: ${msg.message}`; result.errorCode = msg.code; note(`PROTOCOL_ERROR ${msg.code}`); break;
  }
};
ws.onerror = (e) => { note(`ws error ${e.message ?? ''}`); if (!result.error) { result.error = 'ws_error'; result.errorCode = 'ws_error'; } };
ws.onclose = () => { note('ws closed'); setTimeout(() => finish(false), 300); };

function trackNode(n) {
  if (!n || result.nodes.some((x) => x.node_id === n.node_id)) return;
  result.nodes.push({ node_id: n.node_id, node_type: n.node_type, status: n.status, title: n.title, started_at: n.started_at, completed_at: n.completed_at });
}
