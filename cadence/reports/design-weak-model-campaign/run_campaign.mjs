#!/usr/bin/env node
/**
 * Single-sample driver for the Design weak-model campaign.
 *
 * Usage:
 *   node run_campaign.mjs <provider> <shapeId:01..06> <rep> <outRoot> [--dry-run]
 *
 * The driver deliberately seeds the already-confirmed Story Spec fixture directly
 * into Aria's local .aria store.  There is no HTTP endpoint for creating and
 * confirming a fixture without running a Story provider.  It then creates and
 * drives only the Design workspace over WebSocket.
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const CAMPAIGN_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(CAMPAIGN_DIR, '../../..');
const CORPUS_DIR = path.join(CAMPAIGN_DIR, 'corpus');
const PROJECT_ID = process.env.ARIA_PROJECT_ID ?? 'project_0001';
const REPOSITORY_ID = process.env.ARIA_REPOSITORY_ID ?? 'repository_0001';
const ARIA_ROOT = path.resolve(process.env.ARIA_DATA_ROOT ?? path.join(REPO_ROOT, '.aria'));
const BASE = process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317';
const WS_BASE = (process.env.ARIA_WS_BASE_URL ?? BASE.replace(/^http/, 'ws')).replace(/\/$/, '');
const HARD_LIMIT_MS = 1_200_000;
const AUTHOR_LIMIT_MS = 600_000;
const REVIEWER_LIMIT_MS = 600_000;
const FINALIZE_LIMIT_MS = 60_000;
const PROVIDERS = new Set(['claude_code', 'kimi_code', 'pi', 'codex']);
const FEEDBACK = '请根据评审意见修订设计';

function usageAndExit(message, code = 2) {
  console.error(`${message}\nUsage: node run_campaign.mjs <provider:claude_code|kimi_code|pi> <shapeId:01..06> <rep:positive integer> <outRoot> [--dry-run]`);
  process.exit(code);
}

function parseArgs(argv) {
  const args = argv.slice(2);
  const dryRunIndex = args.indexOf('--dry-run');
  const dryRun = dryRunIndex !== -1;
  if (dryRun) args.splice(dryRunIndex, 1);
  if (args.some((arg) => arg.startsWith('--'))) usageAndExit(`Unknown option: ${args.find((arg) => arg.startsWith('--'))}`);
  if (args.length !== 4) usageAndExit('Expected exactly four positional arguments.');

  const [provider, shapeId, rep, outRoot] = args;
  if (!PROVIDERS.has(provider)) usageAndExit(`Unsupported provider: ${provider}`);
  if (!/^(0[1-7])$/.test(shapeId)) usageAndExit(`shapeId must be 01..07, got: ${shapeId}`);
  if (!/^[1-9][0-9]*$/.test(rep)) usageAndExit(`rep must be a positive integer, got: ${rep}`);
  if (!outRoot.trim()) usageAndExit('outRoot must not be empty.');
  return { provider, shapeId, rep, outRoot: path.resolve(outRoot), dryRun };
}

function sha256(text) {
  return crypto.createHash('sha256').update(text, 'utf8').digest('hex');
}

function readDigestLedger() {
  const ledger = new Map();
  const digestPath = path.join(CORPUS_DIR, 'digests.txt');
  const content = fs.readFileSync(digestPath, 'utf8');
  for (const line of content.split(/\r?\n/)) {
    const match = line.match(/^([a-f0-9]{64})\s+\*?(.+?)\s*$/i);
    if (match) ledger.set(match[2], match[1].toLowerCase());
  }
  return ledger;
}

function loadFrozenCorpus(shapeId) {
  const ledger = readDigestLedger();
  const designFile = [...ledger.keys()].find((name) => name.startsWith(`${shapeId}-`) && !name.endsWith('-story-fixture.md'));
  const storyFile = `${shapeId}-story-fixture.md`;
  if (!designFile || !ledger.has(storyFile)) {
    throw new Error(`frozen corpus ledger lacks design/story pair for shape ${shapeId}`);
  }

  const readVerified = (fileName) => {
    const filePath = path.join(CORPUS_DIR, fileName);
    const content = fs.readFileSync(filePath, 'utf8');
    const expected = ledger.get(fileName);
    const actual = sha256(content);
    if (actual !== expected) {
      throw new Error(`corpus digest mismatch for ${fileName}: expected ${expected}, got ${actual}`);
    }
    return { fileName, filePath, content, digest: actual };
  };

  return { design: readVerified(designFile), story: readVerified(storyFile) };
}

function modelIdentity(provider) {
  // The provider APIs do not expose a model/version identifier.  Preserve this
  // explicit CLI-name placeholder until runtime metadata is available.
  return {
    provider,
    model: `${provider}-cli`,
    model_version: `${provider}-cli-version-placeholder`,
  };
}

function boundaryKind(shapeId) {
  if (shapeId === '04') return 'abstract-positive';
  if (shapeId === '05') return 'violation-negative';
  return null;
}

function isoNow() {
  return new Date().toISOString();
}

function json(value) {
  return JSON.stringify(value, null, 2);
}

function writeJsonExclusive(filePath, value) {
  fs.writeFileSync(filePath, json(value), { encoding: 'utf8', flag: 'wx' });
}

function seedConfirmedStorySpec({ issueId, title, fixtureMarkdown }) {
  // `LifecycleStore::create_story_spec` starts at story_spec_0001 for each issue.
  // The driver creates a fresh issue, so its fixture has the same stable id and
  // format as a store-created record plus an already persisted version.
  const storySpecId = 'story_spec_0001';
  const issueRoot = path.join(ARIA_ROOT, 'projects', PROJECT_ID, 'issues', issueId);
  const storySpecRoot = path.join(issueRoot, 'story-specs');
  const versionRoot = path.join(issueRoot, 'versions', storySpecId);
  const now = isoNow();
  const storyPath = path.join(storySpecRoot, `${storySpecId}.json`);
  const versionPath = path.join(versionRoot, 'version_0001.json');

  if (!fs.existsSync(path.join(issueRoot, 'issue.json'))) {
    throw new Error(`cannot seed fixture: issue store path is absent: ${issueRoot}`);
  }
  fs.mkdirSync(storySpecRoot, { recursive: true });
  fs.mkdirSync(versionRoot, { recursive: true });

  // Version is written before its visible spec record, so a concurrent lifecycle
  // listing cannot observe a current_version that has no backing version file.
  writeJsonExclusive(versionPath, {
    id: 'version_0001',
    project_id: PROJECT_ID,
    issue_id: issueId,
    entity_id: storySpecId,
    version: 1,
    markdown: fixtureMarkdown,
    provider_run_refs: [],
    review_refs: [],
    confirmed_by: 'campaign_fixture',
    created_at: now,
  });
  writeJsonExclusive(storyPath, {
    id: storySpecId,
    project_id: PROJECT_ID,
    issue_id: issueId,
    repository_id: REPOSITORY_ID,
    title,
    involved_repository_ids: [],
    current_version: 1,
    confirmation_status: 'confirmed',
    created_at: now,
    updated_at: now,
  });
  return storySpecId;
}

function extractId(value, keys) {
  for (const key of keys) {
    const found = key.split('.').reduce((cursor, part) => cursor?.[part], value);
    if (typeof found === 'string' && found) return found;
  }
  return null;
}

function errorText(error) {
  return error instanceof Error ? error.message : String(error);
}

async function parseResponse(response, label) {
  const text = await response.text();
  let body;
  try {
    body = text ? JSON.parse(text) : {};
  } catch {
    body = { raw: text.slice(0, 1_000) };
  }
  if (!response.ok) {
    const detail = typeof body?.message === 'string' ? body.message : text.slice(0, 500);
    throw new Error(`${label} HTTP ${response.status}${detail ? `: ${detail}` : ''}`);
  }
  return body;
}

async function requestJson(url, options, elapsedMs) {
  const remaining = HARD_LIMIT_MS - elapsedMs();
  if (remaining <= 0) throw new Error('driver-timeout before HTTP request');
  const response = await fetch(url, {
    ...options,
    signal: AbortSignal.timeout(Math.min(remaining, 60_000)),
  });
  return parseResponse(response, options.label ?? options.method ?? 'request');
}

function firstMarkdown(message) {
  return [
    message.markdown,
    message.artifact?.markdown,
    message.payload?.markdown,
  ].find((candidate) => typeof candidate === 'string' && candidate.trim()) ?? null;
}

function findTokenUsage(value, observations, source) {
  if (!value || typeof value !== 'object') return;
  if (Array.isArray(value)) {
    for (const item of value) findTokenUsage(item, observations, source);
    return;
  }

  const input = Number.isInteger(value.input_tokens)
    ? value.input_tokens
    : Number.isInteger(value.prompt_tokens)
      ? value.prompt_tokens
      : null;
  const cacheRead = Number.isInteger(value.cache_read_tokens)
    ? value.cache_read_tokens
    : Number.isInteger(value.cache_read_input_tokens)
      ? value.cache_read_input_tokens
      : null;
  if ((input !== null && input >= 0) || (cacheRead !== null && cacheRead >= 0)) {
    observations.push({
      source,
      input_tokens: Math.max(0, input ?? 0),
      cache_read_tokens: Math.max(0, cacheRead ?? 0),
    });
  }
  for (const nested of Object.values(value)) findTokenUsage(nested, observations, source);
}

function usageDisclosure(observations) {
  if (!observations.length) return { usage_unavailable: true };
  return {
    usage: {
      input_tokens: observations.reduce((sum, item) => sum + item.input_tokens, 0),
      cache_read_tokens: observations.reduce((sum, item) => sum + item.cache_read_tokens, 0),
      fresh_or_resume: 'fresh',
    },
  };
}

// §usage-collection：从 WS execution_event（kind=usage）提取 per-role token 用量。
// usage 事件由 provider 完成时上报（claude result.usage / pi get_state cost / codex
// turn usage），output 为 UsageReportData JSON；缺失字段保留 null（「不可用如实记录」）。
// 注意：写入 usage_by_role 而非覆盖 result.usage——后者受 manifest 契约约束
// （usage.input_tokens / usage.cache_read_tokens 聚合形状，validate_manifest.py 校验）。
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

function createResult({ provider, shapeId, rep, designFile, storyFile }) {
  const identity = modelIdentity(provider);
  const result = {
    provider,
    model: identity.model,
    model_version: identity.model_version,
    role_provider: { author: { ...identity }, reviewer: { ...identity } },
    strategy: 'fresh',
    resume_available: false,
    shape_id: shapeId,
    shape_file: designFile,
    story_fixture_file: storyFile,
    repetition: rep,
    case_id: { shape_id: shapeId, repetition_id: Number(rep) },
    startedAt: isoNow(),
    finishedAt: null,
    issueId: null,
    sessionId: null,
    storySpecId: null,
    designSpecId: null,
    finished: false,
    reviewVerdicts: [],
    failureClass: null,
    elapsedSec: 0,
    boundary_kind: boundaryKind(shapeId),
    error: null,
    errorCode: null,
    aborted: false,
    firstArtifactSec: null,
    authorConfirmSec: null,
    reviewCompleteSec: null,
    finalizeSec: null,
    choices: [],
    permissionApprovals: 0,
    nodes: [],
    timeouts: [],
  };
  if (shapeId === '05') result.first_review_verdict = null;
  return result;
}

async function runCampaign({ provider, shapeId, rep, outRoot, corpus }) {
  const outDir = path.join(outRoot, provider, `${shapeId}-rep${rep}`);
  fs.mkdirSync(outDir, { recursive: true });
  const logPath = path.join(outDir, 'ws.jsonl');
  const log = fs.createWriteStream(logPath, { flags: 'a' });
  const wlog = (entry) => log.write(`${JSON.stringify({ t: Date.now(), ...entry })}\n`);
  const note = (message) => {
    console.log(`[${provider}/${shapeId}/r${rep}] ${message}`);
    wlog({ note: message });
  };
  const result = createResult({
    provider,
    shapeId,
    rep,
    designFile: corpus.design.fileName,
    storyFile: corpus.story.fileName,
  });
  const t0 = Date.now();
  const elapsedMs = () => Date.now() - t0;
  const elapsedSec = () => Number((elapsedMs() / 1_000).toFixed(3));
  const usageObservations = [];
  const usageByRole = {};
  let ws = null;
  let finished = false;
  let hardTimer = null;
  let phaseTimer = null;
  let startSent = false;
  let initialReviewSent = false;
  let reviewDone = false;
  let needsFeedbackRevision = false;
  let feedbackRevisionSent = false;
  let finalizeSent = false;

  const writeArtifact = () => {
    if (result.artifactMarkdown) {
      fs.writeFileSync(path.join(outDir, 'artifact.md'), result.artifactMarkdown, 'utf8');
      delete result.artifactMarkdown;
    }
  };
  const clearTimers = () => {
    if (hardTimer) clearTimeout(hardTimer);
    if (phaseTimer) clearTimeout(phaseTimer);
    hardTimer = null;
    phaseTimer = null;
  };
  const armPhase = (phase, durationMs) => {
    if (phaseTimer) clearTimeout(phaseTimer);
    phaseTimer = setTimeout(() => {
      result.timeouts.push({ phase, elapsedSec: elapsedSec() });
      result.failureClass = 'driver-timeout';
      result.error = `${phase} exceeded ${Math.round(durationMs / 1_000)}s driver limit`;
      note(`DRIVER TIMEOUT ${phase}`);
      finish(true, 1);
    }, durationMs);
  };
  const finish = (aborted, code = 0) => {
    if (finished) return;
    finished = true;
    clearTimers();
    if (aborted) result.aborted = true;
    if (result.finished) {
      result.failureClass = 'completed';
      result.error = null;
    } else if (!result.failureClass) {
      result.failureClass = result.error ? 'driver_error' : 'incomplete';
    }
    Object.assign(result, usageDisclosure(usageObservations));
    result.usage_by_role = Object.keys(usageByRole).length
      ? usageByRole
      : { usage_unavailable: true };
    result.finishedAt = isoNow();
    result.elapsedSec = elapsedSec();
    writeArtifact();
    // D05 判例送达取证：从 timeline detail 读 reviewer prompt，断言含判例 marker（§1b 归因）
    if (shapeId === '05') {
      try {
        const detDir = path.join(ARIA_ROOT, 'projects', PROJECT_ID, 'issues', result.issueId,
          'workspace-timelines', result.sessionId, 'timeline_node_details');
        let delivered = false; let reviewerNode = null;
        for (const f of fs.readdirSync(detDir)) {
          const d = JSON.parse(fs.readFileSync(path.join(detDir, f), 'utf8'));
          if (d.node_type === 'reviewer_run') { reviewerNode = d; break; }
        }
        const prompt = String(reviewerNode?.prompt ?? '');
        delivered = prompt.includes('[design_reviewer_boundary_examples]');
        result.boundary_example_delivered = delivered;
        result.reviewer_prompt_chars = prompt.length;
        if (!delivered) note(`判例未送达! reviewer prompt ${prompt.length} 字符不含 marker`);
      } catch (e) { result.boundary_example_delivered = null; note(`判例取证失败: ${e.message}`); }
    }
    fs.writeFileSync(path.join(outDir, 'result.json'), json(result));
    log.end();
    try { ws?.close(); } catch { /* socket may already be closed */ }
    console.log(json(result));
    setTimeout(() => process.exit(code), 100);
  };
  const fail = (failureClass, error, code = 1) => {
    if (!result.failureClass) result.failureClass = failureClass;
    if (!result.error) result.error = errorText(error);
    note(`FAIL ${result.failureClass}: ${result.error}`);
    finish(true, code);
  };

  // The hard cap begins before issue creation, so fixture seeding and API setup
  // cannot extend a nominal 600-second sample beyond the campaign contract.
  hardTimer = setTimeout(() => {
    result.timeouts.push({ phase: 'hard_cap', elapsedSec: elapsedSec() });
    result.failureClass = 'driver-timeout';
    result.error = 'hard_cap exceeded 600s driver limit';
    note('DRIVER TIMEOUT hard_cap');
    finish(true, 1);
  }, HARD_LIMIT_MS);

  try {
    const issue = await requestJson(
      `${BASE}/api/projects/${PROJECT_ID}/issues`,
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          title: `Design campaign ${provider} ${shapeId} r${rep}`,
          description: corpus.design.content,
          change_id: `design-campaign-${provider}-${shapeId}-r${rep}`,
          repository_id: REPOSITORY_ID,
        }),
        label: 'create issue',
      },
      elapsedMs,
    );
    result.issueId = extractId(issue, ['issue_id', 'issue.issue_id']);
    if (!result.issueId) throw new Error('create issue response omitted issue_id');
    note(`issue ${result.issueId}`);

    result.storySpecId = seedConfirmedStorySpec({
      issueId: result.issueId,
      title: `Design campaign ${shapeId} confirmed Story Spec fixture`,
      fixtureMarkdown: corpus.story.content,
    });
    note(`seeded confirmed story ${result.storySpecId}`);

    const generation = await requestJson(
      `${BASE}/api/projects/${PROJECT_ID}/issues/${result.issueId}/design-specs:generate`,
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          title: `Design campaign ${provider} ${shapeId} r${rep}`,
          story_spec_ids: [result.storySpecId],
          author_provider: provider,
          reviewer_provider: provider,
          review_rounds: 1,
          superpowers_enabled: true,
          openspec_enabled: true,
        }),
        label: 'generate design spec',
      },
      elapsedMs,
    );
    result.sessionId = extractId(generation, [
      'workspace_session.workspace_session_id',
      'workspace_session.session_id',
      'workspace_session.id',
      'session_id',
    ]);
    result.designSpecId = extractId(generation, [
      'design_specs.0.design_spec_id',
      'design_spec.design_spec_id',
      'design_spec_id',
    ]);
    if (!result.sessionId || !result.designSpecId) {
      throw new Error('design-specs:generate response omitted workspace session or design spec id');
    }
    note(`design ${result.designSpecId}; session ${result.sessionId}`);

    if (elapsedMs() >= HARD_LIMIT_MS) throw new Error('driver-timeout before WebSocket connection');
    ws = new WebSocket(`${WS_BASE}/api/workspace-sessions/${result.sessionId}/ws`);
    const send = (message) => {
      wlog({ out: message });
      ws.send(JSON.stringify(message));
    };
    const trackNode = (node) => {
      if (!node?.node_id) return;
      const existing = result.nodes.find((entry) => entry.node_id === node.node_id);
      const summary = node.summary ?? null;
      if (existing) {
        existing.status = node.status ?? existing.status;
        existing.summary = summary ?? existing.summary;
        existing.completed_at = node.completed_at ?? existing.completed_at;
        return;
      }
      result.nodes.push({
        node_id: node.node_id,
        node_type: node.node_type ?? null,
        status: node.status ?? null,
        title: node.title ?? null,
        summary,
        started_at: node.started_at ?? null,
        completed_at: node.completed_at ?? null,
      });
    };
    const handleStage = (stage) => {
      if (stage === 'prepare_context' && !startSent) {
        startSent = true;
        armPhase('author_generation', AUTHOR_LIMIT_MS);
        note('sending start_generation');
        send({
          type: 'start_generation',
          provider_config: {
            author: provider,
            reviewer: provider,
            review_rounds: 1,
            permission_modes: { author: 'auto', reviewer: 'auto' },
          },
          reviewer_enabled: true,
        });
        return;
      }
      if (stage === 'author_confirm' && startSent) {
        if (!initialReviewSent) {
          initialReviewSent = true;
          result.authorConfirmSec ??= elapsedSec();
          if (result.firstArtifactSec === null) result.firstArtifactSec = elapsedSec();
          armPhase('review_generation', REVIEWER_LIMIT_MS);
          note('author_confirm -> accept_with_review');
          send({ type: 'author_decision', decision: 'accept_with_review' });
        } else if (needsFeedbackRevision && !feedbackRevisionSent) {
          feedbackRevisionSent = true;
          armPhase('author_revision', AUTHOR_LIMIT_MS);
          note('review feedback -> author revision');
          send({ type: 'author_decision', decision: { revise: { feedback: FEEDBACK } } });
        } else if (reviewDone && (!needsFeedbackRevision || feedbackRevisionSent) && !finalizeSent) {
          finalizeSent = true;
          result.finalizeSec = elapsedSec();
          armPhase('finalize', FINALIZE_LIMIT_MS);
          note('author_confirm -> accept_finalize');
          send({ type: 'author_decision', decision: 'accept_finalize' });
        }
        return;
      }
      if (stage === 'completed') {
        result.finished = true;
        note('terminal stage completed');
        setTimeout(() => finish(false, 0), 150);
      }
    };

    ws.onopen = () => {
      note('ws open');
      send({ type: 'hello', session_id: result.sessionId, last_seen_node_id: null });
    };
    ws.onmessage = (event) => {
      let message;
      try {
        message = JSON.parse(event.data);
      } catch {
        wlog({ malformed: String(event.data).slice(0, 500) });
        return;
      }
      findTokenUsage(message, usageObservations, message.type ?? 'unknown');
      collectUsageByRole(message, usageByRole);
      const markdown = firstMarkdown(message);
      if (markdown) {
        result.artifactMarkdown = markdown;
        if (result.firstArtifactSec === null) result.firstArtifactSec = elapsedSec();
      }
      const snapshotNodes = message.timeline_nodes;
      if (Array.isArray(snapshotNodes)) snapshotNodes.forEach(trackNode);
      if (message.type === 'timeline_node_created') trackNode(message.node);
      if (message.type === 'timeline_node_updated') {
        trackNode({
          node_id: message.node_id,
          status: message.status,
          summary: message.summary,
          completed_at: message.completed_at,
        });
      }
      wlog({ in: { type: message.type, stage: message.stage, verdict: message.verdict } });

      switch (message.type) {
        case 'session_state':
        case 'stage_change':
          handleStage(message.stage);
          break;
        case 'artifact_update':
          result.artifactVersion = message.version ?? result.artifactVersion;
          break;
        case 'choice_request': {
          const options = message.options ?? [];
          // 工具阻塞类 choice：优先选「跳过/继续」语义选项（仅当问题含工具/权限/阻塞特征）；设计选型类 choice（如 JSON vs MessagePack）正常选首项
          const skipish = (o) => /跳过|继续|忽略|skip|continue/i.test(`${o.label ?? ''}${o.description ?? ''}`);
          const questionText = `${message.prompt ?? ''}${JSON.stringify(message.questions ?? [])}`;
          const isBlockerChoice = /工具|权限|阻塞|读取|read|gate|门禁/i.test(questionText);
          const preferred = isBlockerChoice ? options.find(skipish) : undefined;
          const answers = (message.questions ?? []).map((question) => ({
            question_id: question.question_id ?? question.id,
            selected_option_ids: question.options?.[0]?.id ? [question.options[0].id] : [],
            free_text: null,
          }));
          const selected = preferred?.id ? [preferred.id] : options[0]?.id ? [options[0].id] : answers.flatMap((answer) => answer.selected_option_ids);
          const preferredLabels = options.map((o) => o.label ?? '').filter(Boolean);
          result.choices.push({ elapsedSec: elapsedSec(), picked: selected, labels: preferredLabels });
          note(`choice -> ${JSON.stringify(selected)}`);
          send({
            type: 'choice_response',
            id: message.id,
            selected_option_ids: selected,
            free_text: null,
            answers,
          });
          break;
        }
        case 'permission_request':
          result.permissionApprovals += 1;
          send({ type: 'permission_response', id: message.id, approved: true, reason: null });
          break;
        case 'review_complete': {
          const verdict = message.verdict;
          reviewDone = true;
          result.reviewCompleteSec = elapsedSec();
          result.reviewVerdicts.push({ verdict, el: elapsedSec() });
          if (result.reviewVerdicts.length === 1 && shapeId === '05') result.first_review_verdict = verdict;
          needsFeedbackRevision = verdict === 'revise' || verdict === 'needs_human';
          armPhase(needsFeedbackRevision ? 'author_revision_wait' : 'finalize_wait', AUTHOR_LIMIT_MS);
          note(`review_complete verdict=${verdict}`);
          break;
        }
        case 'error':
          result.error = message.message ?? 'workspace error';
          result.errorCode = 'workspace_error';
          result.failureClass ??= 'provider_error';
          note(`ERROR ${result.error}`);
          break;
        case 'protocol_error':
          result.error = `${message.code ?? 'protocol_error'}: ${message.message ?? ''}`;
          result.errorCode = message.code ?? 'protocol_error';
          result.failureClass ??= 'protocol_error';
          note(`PROTOCOL_ERROR ${result.error}`);
          break;
        default:
          break;
      }
    };
    ws.onerror = (event) => {
      result.error ??= event.message ?? 'ws_error';
      result.errorCode ??= 'ws_error';
      result.failureClass ??= 'driver_transport_error';
      note(`WS ERROR ${result.error}`);
    };
    ws.onclose = (ev) => {
      note(`ws closed code=${ev?.code} reason=${JSON.stringify(ev?.reason ?? '')} wasClean=${ev?.wasClean}`);
      if (!finished) setTimeout(() => finish(false, result.finished ? 0 : 1), 100);
    };
  } catch (error) {
    const message = errorText(error);
    const timedOut = /driver-timeout|timeout|timed out/i.test(message);
    fail(timedOut ? 'driver-timeout' : 'driver_setup_error', message, 1);
  }
}

async function main() {
  const options = parseArgs(process.argv);
  let corpus;
  try {
    corpus = loadFrozenCorpus(options.shapeId);
  } catch (error) {
    console.error(`Corpus validation failed: ${errorText(error)}`);
    process.exit(2);
  }

  if (options.dryRun) {
    console.log(json({
      dry_run: true,
      provider: options.provider,
      shape_id: options.shapeId,
      repetition: options.rep,
      out_root: options.outRoot,
      design_file: corpus.design.fileName,
      story_fixture_file: corpus.story.fileName,
      design_digest: corpus.design.digest,
      story_digest: corpus.story.digest,
      seed_method: 'filesystem',
      no_http_or_websocket_requests: true,
    }));
    return;
  }

  await runCampaign({ ...options, corpus });
}

await main();
