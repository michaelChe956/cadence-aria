#!/usr/bin/env node
/**
 * Work Item Plan campaign 单样本驱动器。
 * 仅在显式省略 --dry-run 时才会调用 Aria HTTP/WS 与 provider。
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const CAMPAIGN_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(CAMPAIGN_DIR, '../../..');
const FIXTURES_DIR = path.join(CAMPAIGN_DIR, 'fixtures');
const DESCRIPTION_FILE = path.join(
  REPO_ROOT,
  'cadence/reports/design-weak-model-campaign/corpus/07-fullstack-levels.md',
);
const DESCRIPTION_LEDGER = path.join(
  REPO_ROOT,
  'cadence/reports/design-weak-model-campaign/corpus/digests.txt',
);
const PROVIDERS = new Set(['claude_code', 'kimi_code', 'pi', 'codex']);
const FEEDBACK = '请根据评审意见修订';
// 阶段 1：策略层接在 legacy 路径上，flow_kind 期望为 legacy；阶段 2 切 single_candidate。
const EXPECTED_FLOW_KIND = process.env.ARIA_EXPECTED_FLOW_KIND ?? 'legacy';
const CONFIGURED_RUN_POLICY = process.env.ARIA_RUN_POLICY ?? 'auto_if_valid';
const MANUAL_REPAIR_BUDGET = 3;
const MAX_AUTOMATIC_REPAIRS_PER_CYCLE = 1;
const HARD_LIMIT_MS = Number(
  process.env.ARIA_WORKITEM_HARD_TIMEOUT_MS ?? process.env.ARIA_HARD_TIMEOUT_MS ?? 35 * 60_000,
);
const PROJECT_ID = process.env.ARIA_PROJECT_ID ?? 'project_0001';
const REPOSITORY_ID = process.env.ARIA_REPOSITORY_ID ?? 'repository_0001';
const BASE = (process.env.ARIA_BASE_URL ?? 'http://127.0.0.1:4317').replace(/\/$/, '');
const WS_BASE = (process.env.ARIA_WS_BASE_URL ?? BASE.replace(/^http/, 'ws')).replace(/\/$/, '');
const ARIA_ROOT = path.resolve(process.env.ARIA_DATA_ROOT ?? path.join(REPO_ROOT, '.aria'));
const FIXTURE_FILES = [
  'story_spec_0001.json',
  'story_version_0001.json',
  'design_spec_0001.json',
  'design_version_0001.json',
];
const AUTHOR_CONFIRM_NODE_TYPES = new Set([
  'author_confirm',
  'work_item_plan_outline_confirm',
  'work_item_generation_mode',
  'work_item_draft_confirm',
  'work_item_batch_confirm',
  'work_item_plan_compile_recovery',
  'work_item_plan_context_blocker',
]);
const PREPARE_OPTIONS = {
  story_spec_ids: ['story_spec_0001'],
  design_spec_ids: ['design_spec_0001'],
  author_provider: null,
  reviewer_provider: null,
  review_rounds: 1,
  // 与 provider_workspace_config 的 UI 默认值一致，显式写入以固定采样条件。
  superpowers_enabled: true,
  openspec_enabled: true,
  // PrepareWorkItemPlanRequest 使用 snake_case serde 名称；campaign 只接受单候选协议。
  run_policy: CONFIGURED_RUN_POLICY,
  include_integration_tests: true,
  include_e2e_tests: false,
  force_frontend_backend_split: true,
  require_execution_plan_confirm: false,
};

function usageAndExit(message, code = 2) {
  console.error(`${message}\nUsage: node workitem_run_campaign.mjs <provider:claude_code|kimi_code|pi|codex> <rep:positive integer> <outRoot> [--dry-run]`);
  process.exit(code);
}

function parseArgs(argv) {
  const args = argv.slice(2);
  const dryIndex = args.indexOf('--dry-run');
  const dryRun = dryIndex !== -1;
  if (dryRun) args.splice(dryIndex, 1);
  const unknown = args.find((arg) => arg.startsWith('--'));
  if (unknown) usageAndExit(`未知选项: ${unknown}`);
  if (args.length !== 3) usageAndExit('必须提供三个位置参数。');
  const [provider, rep, outRoot] = args;
  if (!PROVIDERS.has(provider)) usageAndExit(`不支持的 provider: ${provider}`);
  if (!/^[1-9][0-9]*$/.test(rep)) usageAndExit(`rep 必须是正整数: ${rep}`);
  if (!outRoot.trim()) usageAndExit('outRoot 不能为空。');
  if (!Number.isFinite(HARD_LIMIT_MS) || HARD_LIMIT_MS <= 0) {
    usageAndExit('ARIA_WORKITEM_HARD_TIMEOUT_MS 必须是正整数毫秒数。');
  }
  if (!['auto_if_valid', 'interactive'].includes(CONFIGURED_RUN_POLICY)) {
    usageAndExit(`ARIA_RUN_POLICY 不受支持: ${CONFIGURED_RUN_POLICY}`);
  }
  const existingSessionId = process.env.ARIA_EXISTING_SESSION?.trim() || null;
  const feedbackFile = process.env.ARIA_FEEDBACK_FILE?.trim() || null;
  return {
    provider,
    rep,
    outRoot: path.resolve(outRoot),
    dryRun,
    existingSessionId,
    feedbackFile,
    runPolicy: CONFIGURED_RUN_POLICY,
  };
}

function json(value) {
  return JSON.stringify(value, null, 2);
}

function now() {
  return new Date().toISOString();
}

function sha256(text) {
  return crypto.createHash('sha256').update(text, 'utf8').digest('hex');
}

function errorText(error) {
  return error instanceof Error ? error.message : String(error);
}

function readLedger(filePath) {
  const ledger = new Map();
  for (const line of fs.readFileSync(filePath, 'utf8').split(/\r?\n/)) {
    const matched = line.match(/^([a-f0-9]{64})\s+\*?(.+?)\s*$/i);
    if (matched) ledger.set(matched[2], matched[1].toLowerCase());
  }
  return ledger;
}

function validateFixtures() {
  const ledger = readLedger(path.join(FIXTURES_DIR, 'digests.txt'));
  const digests = {};
  for (const fileName of FIXTURE_FILES) {
    const expected = ledger.get(fileName);
    if (!expected) throw new Error(`fixture digest ledger 缺少 ${fileName}`);
    const filePath = path.join(FIXTURES_DIR, fileName);
    const source = fs.readFileSync(filePath, 'utf8');
    const actual = sha256(source);
    if (actual !== expected) {
      throw new Error(`fixture digest 不匹配: ${fileName}; expected=${expected}; actual=${actual}`);
    }
    digests[fileName] = actual;
  }
  const unexpected = [...ledger.keys()].filter((fileName) => !FIXTURE_FILES.includes(fileName));
  if (unexpected.length) throw new Error(`fixture digest ledger 包含未受支持的文件: ${unexpected.join(', ')}`);
  return digests;
}

function loadDescription() {
  const content = fs.readFileSync(DESCRIPTION_FILE, 'utf8');
  const ledger = readLedger(DESCRIPTION_LEDGER);
  const expected = ledger.get('07-fullstack-levels.md');
  if (!expected) throw new Error('案例语料 ledger 缺少 07-fullstack-levels.md');
  const digest = sha256(content);
  if (digest !== expected) {
    throw new Error(`案例语料 digest 不匹配: expected=${expected}; actual=${digest}`);
  }
  return { content, digest };
}

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
    throw new Error(`无法 seed fixture：issue store 不存在: ${issueRoot}`);
  }
  const timestamp = now();
  const destinations = {
    story_spec_0001: path.join(issueRoot, 'story-specs', 'story_spec_0001.json'),
    design_spec_0001: path.join(issueRoot, 'design-specs', 'design_spec_0001.json'),
    story_version_0001: path.join(issueRoot, 'versions', 'story_spec_0001', 'version_0001.json'),
    design_version_0001: path.join(issueRoot, 'versions', 'design_spec_0001', 'version_0001.json'),
  };
  // 先落版本，避免 current_version 指向尚未落盘的版本记录。
  for (const name of ['story_version_0001', 'design_version_0001', 'story_spec_0001', 'design_spec_0001']) {
    const fixture = JSON.parse(fs.readFileSync(path.join(FIXTURES_DIR, `${name}.json`), 'utf8'));
    writeJsonExclusive(destinations[name], rewriteFixture(fixture, issueId, timestamp));
  }
  return { issueRoot, seededAt: timestamp, destinations };
}

function extractId(value, paths) {
  for (const dottedPath of paths) {
    const found = dottedPath.split('.').reduce((cursor, key) => cursor?.[key], value);
    if (typeof found === 'string' && found) return found;
  }
  return null;
}

async function parseResponse(response, label) {
  const text = await response.text();
  let body = {};
  try {
    body = text ? JSON.parse(text) : {};
  } catch {
    body = { raw: text.slice(0, 1_000) };
  }
  if (!response.ok) {
    const detail = typeof body.message === 'string' ? body.message : text.slice(0, 500);
    throw new Error(`${label} HTTP ${response.status}${detail ? `: ${detail}` : ''}`);
  }
  return body;
}

async function requestJson(url, options, elapsedMs) {
  const remaining = HARD_LIMIT_MS - elapsedMs();
  if (remaining <= 0) throw new Error('hard-timeout before HTTP request');
  const response = await fetch(url, {
    ...options,
    signal: AbortSignal.timeout(Math.min(remaining, 60_000)),
  });
  return parseResponse(response, options.label ?? options.method ?? 'request');
}

// 从 WS execution_event（kind=usage）递归提取 UsageReportData JSON 的按角色 token 用量。
function collectUsageByRole(value, byRole) {
  if (!value || typeof value !== 'object') return;
  if (Array.isArray(value)) {
    value.forEach((item) => collectUsageByRole(item, byRole));
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
      // usage output 格式错误时失败关闭于采集：不从其他字段推断 token 用量。
    }
    return;
  }
  Object.values(value).forEach((item) => collectUsageByRole(item, byRole));
}

function collectFindings(value, findings) {
  if (!value || typeof value !== 'object') return;
  if (Array.isArray(value)) {
    value.forEach((item) => collectFindings(item, findings));
    return;
  }
  if (Array.isArray(value.validator_findings)) findings.push(...value.validator_findings);
  Object.values(value).forEach((item) => collectFindings(item, findings));
}

function isRecord(value) {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function nonNegativeInteger(value, path) {
  if (!Number.isInteger(value) || value < 0) {
    throw new Error(`${path} 必须是非负整数`);
  }
  return value;
}

function normalizedPolicyDiagnostics(value) {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new Error('$.policy_diagnostics 必须是数组');
  return value.map((diagnostic, index) => {
    if (!isRecord(diagnostic) || typeof diagnostic.code !== 'string' || !diagnostic.code) {
      throw new Error(`$.policy_diagnostics[${index}].code 必须是非空字符串`);
    }
    if (typeof diagnostic.message !== 'string') {
      throw new Error(`$.policy_diagnostics[${index}].message 必须是字符串`);
    }
    if (diagnostic.field !== null && diagnostic.field !== undefined && typeof diagnostic.field !== 'string') {
      throw new Error(`$.policy_diagnostics[${index}].field 必须是字符串或 null`);
    }
    return {
      code: diagnostic.code,
      message: diagnostic.message,
      field: diagnostic.field ?? null,
    };
  });
}

function normalizedRunHistory(value) {
  if (!isRecord(value)) throw new Error('$.run_history 必须是对象');
  const runHistory = {
    repairs_used: nonNegativeInteger(value.repairs_used, '$.run_history.repairs_used'),
    manual_repairs_used: nonNegativeInteger(value.manual_repairs_used, '$.run_history.manual_repairs_used'),
    transitions_used: nonNegativeInteger(value.transitions_used, '$.run_history.transitions_used'),
    initial_review_count: nonNegativeInteger(value.initial_review_count, '$.run_history.initial_review_count'),
    verification_review_count: nonNegativeInteger(value.verification_review_count, '$.run_history.verification_review_count'),
    review_cycles: {},
  };
  if (!isRecord(value.review_cycles)) throw new Error('$.run_history.review_cycles 必须是对象');
  for (const [cycleId, cycle] of Object.entries(value.review_cycles)) {
    if (!isRecord(cycle)) throw new Error(`$.run_history.review_cycles.${cycleId} 必须是对象`);
    const normalized = {
      repairs_used: nonNegativeInteger(cycle.repairs_used, `$.run_history.review_cycles.${cycleId}.repairs_used`),
      initial_count: nonNegativeInteger(cycle.initial_count, `$.run_history.review_cycles.${cycleId}.initial_count`),
      verification_count: nonNegativeInteger(cycle.verification_count, `$.run_history.review_cycles.${cycleId}.verification_count`),
    };
    if (normalized.initial_count > 1) {
      throw new Error(`$.run_history.review_cycles.${cycleId}.initial_count 必须至多为 1`);
    }
    if (normalized.verification_count > 1) {
      throw new Error(`$.run_history.review_cycles.${cycleId}.verification_count 必须至多为 1`);
    }
    if (normalized.repairs_used > MAX_AUTOMATIC_REPAIRS_PER_CYCLE) {
      throw new Error(`$.run_history.review_cycles.${cycleId}.repairs_used 超出每 cycle 自动返修预算`);
    }
    runHistory.review_cycles[cycleId] = normalized;
  }
  return runHistory;
}

function providerStartCount(value) {
  if (value === undefined) return 0;
  if (!Array.isArray(value)) throw new Error('$.provider_start_ledger 必须是数组');
  const keys = new Set();
  value.forEach((entry, index) => {
    if (!isRecord(entry) || typeof entry.provider_start_idempotency_key !== 'string' || !entry.provider_start_idempotency_key) {
      throw new Error(`$.provider_start_ledger[${index}].provider_start_idempotency_key 必须是非空字符串`);
    }
    if (typeof entry.started !== 'boolean') {
      throw new Error(`$.provider_start_ledger[${index}].started 必须是布尔值`);
    }
    if (entry.started) keys.add(entry.provider_start_idempotency_key);
  });
  return keys.size;
}

// 仅从 SessionState 的 durable 字段读取策略、计数和 provider 启动账本；不使用 stage/event 推断。
function sessionStateProtocol(message, expectedRunPolicy = CONFIGURED_RUN_POLICY) {
  if (!isRecord(message) || message.type !== 'session_state') {
    throw new Error('首条状态消息必须是 session_state');
  }
  if (message.flow_kind !== EXPECTED_FLOW_KIND) {
    throw new Error(`$.flow_kind 必须为 ${EXPECTED_FLOW_KIND}，实际为 ${String(message.flow_kind)}`);
  }
  if (message.run_policy !== expectedRunPolicy) {
    throw new Error(`$.run_policy 必须为 ${expectedRunPolicy}，实际为 ${String(message.run_policy)}`);
  }
  if (typeof message.session_status !== 'string' || !message.session_status) {
    throw new Error('$.session_status 必须是非空字符串');
  }
  return {
    session_status: message.session_status,
    flow_kind: message.flow_kind,
    run_policy: message.run_policy,
    run_history: normalizedRunHistory(message.run_history),
    policy_diagnostics: normalizedPolicyDiagnostics(message.policy_diagnostics),
    human_gate_snapshot: message.human_gate_snapshot ?? null,
    provider_start_count: providerStartCount(message.provider_start_ledger),
  };
}

// `terminal` 表示服务端已持久化的合法终局，driver 应写结果并结束，不能再等待 WS 关闭。
function terminalSessionAction(
  message,
  awaitingReviewNeedsHumanTerminal = false,
  terminalReadback = false,
) {
  const diagnostics = normalizedPolicyDiagnostics(message?.policy_diagnostics);
  if (message?.session_status === 'confirmed') return { kind: 'complete' };
  if (message?.session_status === 'stopped_needs_human') {
    return { kind: 'terminal', failureClass: 'stopped_needs_human' };
  }
  if (message?.session_status === 'failed') {
    return { kind: 'terminal', failureClass: 'policy_failed' };
  }
  if (
    message?.session_status === 'waiting_for_human'
    && (awaitingReviewNeedsHumanTerminal || terminalReadback)
  ) {
    return { kind: 'terminal', failureClass: 'awaiting_human' };
  }
  const fatalDiagnostic = diagnostics.find((diagnostic) => (
    diagnostic.code === 'unknown_finding_category' || diagnostic.code === 'unknown_class_hint'
  ));
  if (fatalDiagnostic) return { kind: 'fail', failureClass: fatalDiagnostic.code };
  return { kind: 'continue' };
}

function reviewCycleId(message) {
  const review = message?.work_item_plan_review;
  if (!isRecord(review)) return null;
  if (review.review_scope === 'outline' && typeof review.target_outline_id === 'string' && review.target_outline_id) {
    return `outline:${review.target_outline_id}`;
  }
  if (review.review_scope === 'item' && typeof review.target_outline_id === 'string' && review.target_outline_id) {
    return `draft:${review.target_outline_id}`;
  }
  if (review.review_scope === 'batch' && typeof review.batch_id === 'string' && review.batch_id) {
    return `batch:${review.batch_id}`;
  }
  return null;
}

// 每个 durable review cycle 最多一次自动返修；不以 cross_review stage 或其他 cycle 余量推断。
function reviewRepairAction(runHistory, reviewComplete) {
  if (!isRecord(runHistory) || !isRecord(runHistory.review_cycles)) {
    return { kind: 'fail', failureClass: 'durable_run_history_missing' };
  }
  const cycleId = reviewCycleId(reviewComplete);
  if (!cycleId) return { kind: 'fail', failureClass: 'durable_review_cycle_unresolved' };
  const cycle = runHistory.review_cycles[cycleId];
  if (!isRecord(cycle)) return { kind: 'fail', failureClass: 'durable_review_cycle_missing' };
  return cycle.repairs_used < MAX_AUTOMATIC_REPAIRS_PER_CYCLE
    ? { kind: 'continue' }
    : { kind: 'fail', failureClass: 'review_revision_limit' };
}

function prepareOptionsForProvider(provider, runPolicy = CONFIGURED_RUN_POLICY) {
  return {
    ...PREPARE_OPTIONS,
    author_provider: provider,
    reviewer_provider: provider,
    run_policy: runPolicy,
  };
}

function readFeedbackFile(filePath) {
  if (!filePath) return FEEDBACK;
  const feedback = fs.readFileSync(path.resolve(filePath), 'utf8');
  if (!feedback.trim()) throw new Error('ARIA_FEEDBACK_FILE 不能为空');
  return feedback;
}

function humanConfirmRequestChangeMessage(feedback) {
  return {
    type: 'human_confirm',
    decision: 'request-change',
    payload: { description: feedback, source: 'human' },
  };
}

function hasMustFixFindings(reviewComplete, humanGateSnapshot = null) {
  const reviewFindings = Array.isArray(reviewComplete)
    ? reviewComplete
    : Array.isArray(reviewComplete?.findings)
      ? reviewComplete.findings
      : [];
  const gateFindings = Array.isArray(humanGateSnapshot?.findings)
    ? humanGateSnapshot.findings
    : [];
  return [...reviewFindings, ...gateFindings]
    .some((finding) => isRecord(finding) && finding.severity === 'must_fix');
}

function humanConfirmAction({
  activeNodeType,
  reviewVerdict,
  reviewComplete,
  humanGateSnapshot = null,
  manualRepairsUsed = 0,
  manualRepairRequestsSent = 0,
  manualRepairBudget = MANUAL_REPAIR_BUDGET,
}) {
  if (activeNodeType === null) return { kind: 'wait' };
  if (activeNodeType !== 'human_confirm') {
    return { kind: 'fail', failureClass: 'unexpected_human_confirm' };
  }
  // human_confirm 可能由 needs_human 门或复评后的最终批准门触发；只能依据最近一次
  // review verdict 判定，不能用旧 gate snapshot/findings 或是否已经发过人工请求反推。
  if (reviewVerdict === 'pass') return { kind: 'confirm' };
  if (!['needs_human', 'revise'].includes(reviewVerdict)) {
    return { kind: 'fail', failureClass: 'human_confirm_verdict_unknown' };
  }
  if (Math.max(manualRepairsUsed, manualRepairRequestsSent) >= manualRepairBudget) {
    return { kind: 'fail', failureClass: 'manual_budget_exhausted' };
  }
  return { kind: 'request_change' };
}

function findExistingSessionMetadata(sessionId) {
  if (!/^[A-Za-z0-9_-]+$/.test(sessionId)) {
    throw new Error(`ARIA_EXISTING_SESSION 非法: ${sessionId}`);
  }
  const projectsRoot = path.join(ARIA_ROOT, 'projects', PROJECT_ID, 'issues');
  if (!fs.existsSync(projectsRoot)) {
    throw new Error(`无法找到 existing session 的 issue store: ${projectsRoot}`);
  }
  const matches = [];
  for (const issueEntry of fs.readdirSync(projectsRoot, { withFileTypes: true })) {
    if (!issueEntry.isDirectory()) continue;
    const sessionPath = path.join(
      projectsRoot,
      issueEntry.name,
      'workspace-sessions',
      `${sessionId}.json`,
    );
    if (!fs.existsSync(sessionPath)) continue;
    const record = JSON.parse(fs.readFileSync(sessionPath, 'utf8'));
    if (!isRecord(record) || record.id !== sessionId) {
      throw new Error(`existing session 记录身份不匹配: ${sessionPath}`);
    }
    if (record.workspace_type !== 'work_item_plan') {
      throw new Error(`existing session workspace_type 必须为 work_item_plan: ${record.workspace_type}`);
    }
    if (typeof record.issue_id !== 'string' || !record.issue_id
      || typeof record.entity_id !== 'string' || !record.entity_id) {
      throw new Error(`existing session 缺少 issue_id 或 plan_id: ${sessionPath}`);
    }
    const runPolicy = record.run_policy ?? 'interactive';
    if (!['auto_if_valid', 'interactive'].includes(runPolicy)) {
      throw new Error(`existing session run_policy 不受支持: ${runPolicy}`);
    }
    matches.push({
      issueId: record.issue_id,
      planId: record.entity_id,
      runPolicy,
      sessionPath,
    });
  }
  if (matches.length === 0) throw new Error(`找不到 existing session: ${sessionId}`);
  if (matches.length > 1) throw new Error(`existing session 不唯一: ${sessionId}`);
  return matches[0];
}

function firstChoice(message) {
  const options = Array.isArray(message.options) ? message.options : [];
  const blocker = /工具|权限|阻塞|tool|permission|block|gate/i.test(
    `${message.prompt ?? ''}${JSON.stringify(message.questions ?? [])}`,
  );
  const skipOrContinue = blocker
    ? options.find((option) => /跳过|继续|忽略|skip|continue/i.test(`${option.label ?? ''}${option.description ?? ''}`))
    : null;
  const selected = skipOrContinue ?? options[0] ?? null;
  const answers = (message.questions ?? []).map((question) => ({
    question_id: question.question_id ?? question.id,
    selected_option_ids: question.options?.[0]?.id ? [question.options[0].id] : [],
    free_text: null,
  }));
  const ids = selected?.id ? [selected.id] : answers.flatMap((answer) => answer.selected_option_ids);
  return { ids, answers, strategy: skipOrContinue ? 'blocker_skip_or_continue' : 'first_option' };
}

function skipOptionalFindingsOption(options) {
  return options.find((option) => {
    const text = String(option).trim();
    return text === 'skip_optional_findings'
      || /(?:跳过|忽略|skip|ignore|bypass).*(?:可选|optional)/i.test(text);
  }) ?? null;
}

function providerRolesForSelection(message, currentStage) {
  const defaults = message.defaults;
  if (defaults && typeof defaults === 'object' && !Array.isArray(defaults)) {
    const roles = [];
    if (typeof defaults.author === 'string' && defaults.author) roles.push('author');
    if (typeof defaults.reviewer === 'string' && defaults.reviewer) roles.push('reviewer');
    if (roles.length) return { roles, strategy: 'message_defaults', assumption: null };
  }

  const stage = String(message.stage ?? currentStage ?? '').toLowerCase();
  if (['prepare_context', 'running', 'author_confirm', 'revision'].includes(stage)) {
    return { roles: ['author'], strategy: 'stage_author', assumption: null };
  }
  if (['cross_review', 'review_decision'].includes(stage)) {
    return { roles: ['reviewer'], strategy: 'stage_reviewer', assumption: null };
  }
  return {
    roles: ['author'],
    strategy: 'author_fallback',
    assumption: 'provider_select_request 未提供可识别 defaults 或 stage；按保守假设仅选择 author。',
  };
}

function reviewCompleteAction(verdict, runPolicy = CONFIGURED_RUN_POLICY) {
  // interactive 模式把 needs_human 当作人工门输入，等待 human_confirm 节点激活后
  // 发送 RequestChange；auto_if_valid 仍等待服务端 durable terminal。
  if (verdict === 'needs_human' && runPolicy === 'interactive') {
    return { kind: 'human_confirm_request_change' };
  }
  return verdict === 'needs_human'
    ? { kind: 'await_terminal_session_state' }
    : { kind: 'continue' };
}

function reviewFindingsContext(reviewComplete) {
  const requiredActions = (Array.isArray(reviewComplete?.findings) ? reviewComplete.findings : [])
    .map((finding) => (isRecord(finding) ? finding.required_action : null))
    .filter((requiredAction) => typeof requiredAction === 'string' && requiredAction.trim())
    .map((requiredAction) => requiredAction.trim());
  return requiredActions.length ? requiredActions.join('\n') : FEEDBACK;
}

function reviewDecisionAction(message, reviewVerdict, reviewComplete = null) {
  const options = Array.isArray(message.options) ? message.options : [];
  if (options.length) {
    const selected = skipOptionalFindingsOption(options);
    if (selected) {
      return {
        kind: 'respond',
        decision: selected,
        strategy: 'skip_optional_findings_keyword',
        extra_context: null,
      };
    }
    if (reviewVerdict === 'revise' && options.includes('continue_with_context')) {
      return {
        kind: 'respond',
        decision: 'continue_with_context',
        strategy: 'revise_continue_with_context',
        extra_context: reviewFindingsContext(reviewComplete),
      };
    }
    if (options.length === 1 && options[0] === 'continue') {
      return {
        kind: 'respond',
        decision: 'continue',
        strategy: 'revise_continue',
        extra_context: null,
      };
    }
    return { kind: 'fail', strategy: 'unhandled_options' };
  }
  if (reviewVerdict === 'revise') {
    return { kind: 'revise', decision: 'revise-with-context', strategy: 'legacy_revise_without_options' };
  }
  return { kind: 'fail', strategy: 'unhandled_without_options' };
}

function generationModeSelectionForNode(nodeId, respondedNodeIds) {
  if (!nodeId || respondedNodeIds.has(nodeId)) return null;
  respondedNodeIds.add(nodeId);
  return { type: 'select_work_item_generation_mode', mode: 'batch' };
}

function authorConfirmAction(activeNodeType, hasLatestArtifact) {
  if (activeNodeType === null) return { kind: 'wait' };
  if (!AUTHOR_CONFIRM_NODE_TYPES.has(activeNodeType)) {
    return { kind: 'fail', failureClass: 'unexpected_author_confirm' };
  }
  if (activeNodeType === 'author_confirm') return { kind: 'author_accept_with_review' };
  if (activeNodeType === 'work_item_plan_outline_confirm') return { kind: 'author_accept' };
  if (activeNodeType === 'work_item_generation_mode') return { kind: 'select_batch' };
  if (activeNodeType === 'work_item_draft_confirm') {
    return hasLatestArtifact
      ? { kind: 'draft_decision' }
      : { kind: 'fail', failureClass: 'draft_artifact_missing' };
  }
  if (activeNodeType === 'work_item_batch_confirm') {
    return hasLatestArtifact
      ? { kind: 'batch_decision' }
      : { kind: 'fail', failureClass: 'batch_artifact_missing' };
  }
  return { kind: 'fail', failureClass: 'unhandled_work_item_plan_gate' };
}

function resultTemplate(
  provider,
  rep,
  outDir,
  fixtureDigests,
  descriptionDigest,
  runPolicy = CONFIGURED_RUN_POLICY,
) {
  return {
    provider,
    repetition: Number(rep),
    project_id: PROJECT_ID,
    repository_id: REPOSITORY_ID,
    outDir,
    startedAt: now(),
    finishedAt: null,
    elapsedSec: null,
    issue_id: null,
    plan_id: null,
    workspace_session_id: null,
    prepare_options: prepareOptionsForProvider(provider, runPolicy),
    fixture_digests: fixtureDigests,
    description_digest: descriptionDigest,
    stageTimeline: [],
    choices: [],
    provider_selections: [],
    review_decisions: [],
    verdicts: [],
    validator_findings: [],
    work_item_ids: [],
    usage_by_role: { usage_unavailable: true },
    artifacts: [],
    permission_approvals: 0,
    failureClass: null,
    error: null,
    completed: false,
    session_status: null,
    flow_kind: null,
    run_policy: null,
    run_history: null,
    policy_diagnostics: [],
    human_gate_snapshot: null,
    provider_start_count: 0,
    stage_durations_sec: {},
  };
}

async function verifyConfirmedPlan(result, elapsedMs) {
  const lifecycle = await requestJson(
    `${BASE}/api/issues/${encodeURIComponent(result.issue_id)}/lifecycle?project_id=${encodeURIComponent(PROJECT_ID)}`,
    { method: 'GET', label: 'read lifecycle for confirmed plan' },
    elapsedMs,
  );
  const plans = Array.isArray(lifecycle.work_item_plans) ? lifecycle.work_item_plans : [];
  const plan = plans.find((candidate) => (candidate.id ?? candidate.plan_id) === result.plan_id);
  if (!plan) throw new Error(`lifecycle 回读缺少 plan ${result.plan_id}`);
  if (String(plan.status).toLowerCase() !== 'confirmed') {
    throw new Error(`plan ${result.plan_id} 未 Confirmed，实际状态: ${plan.status}`);
  }
  const ids = Array.isArray(plan.work_item_ids) ? plan.work_item_ids : [];
  if (!ids.length) throw new Error(`plan ${result.plan_id} Confirmed 但没有 work items`);
  return { plan, workItemIds: ids, lifecycle };
}

async function runCampaign({
  provider,
  rep,
  outRoot,
  fixtureDigests,
  description,
  existingSessionId = null,
  feedbackFile = null,
  runPolicy = CONFIGURED_RUN_POLICY,
}) {
  const outDir = path.join(outRoot, provider, `rep${rep}`);
  fs.mkdirSync(outDir, { recursive: true });
  const log = fs.createWriteStream(path.join(outDir, 'ws.jsonl'), { flags: 'wx' });
  const writeLog = (entry) => log.write(`${JSON.stringify({ at: now(), ...entry })}\n`);
  let feedback = runPolicy === 'interactive' ? FEEDBACK : FEEDBACK;
  const result = resultTemplate(
    provider,
    rep,
    outDir,
    fixtureDigests ?? {},
    description?.digest ?? null,
    runPolicy,
  );
  const started = Date.now();
  const elapsedMs = () => Date.now() - started;
  const elapsedSec = () => Number((elapsedMs() / 1_000).toFixed(3));
  const usageByRole = {};
  let ws = null;
  let ended = false;
  let hardTimer = null;
  let startSent = false;
  let humanConfirmSent = false;
  let manualRepairRequestsSent = 0;
  let latestArtifact = null;
  let reviewVerdict = null;
  let latestReviewComplete = null;
  let sessionProtocolSeen = false;
  let latestSessionStateMessage = null;
  let completionVerificationStarted = false;
  let awaitingReviewNeedsHumanTerminal = false;
  let durableRunHistory = null;
  let activeNodeType = null;
  let activeNodeId = null;
  let currentStage = null;
  const nodesById = new Map();
  const respondedAuthorNodeIds = new Set();
  const acceptedDrafts = new Set();

  const finish = (exitCode = 0) => {
    if (ended) return;
    ended = true;
    if (hardTimer) clearTimeout(hardTimer);
    if (!result.completed && !result.failureClass) result.failureClass = result.error ? 'driver_error' : 'incomplete';
    result.usage_by_role = Object.keys(usageByRole).length
      ? usageByRole
      : { usage_unavailable: true };
    result.finishedAt = now();
    result.elapsedSec = elapsedSec();
    result.stage_durations_sec = Object.fromEntries(result.stageTimeline.map((entry, index) => [
      `${index}:${entry.stage}`,
      Math.round(((result.stageTimeline[index + 1]?.elapsedSec ?? result.elapsedSec) - entry.elapsedSec) * 1_000) / 1_000,
    ]));
    fs.writeFileSync(path.join(outDir, 'result.json'), json(result), 'utf8');
    try { ws?.close(); } catch { /* 连接已关闭时无需处理。 */ }
    log.end(() => process.exit(exitCode));
  };
  const fail = (failureClass, error, exitCode = 1) => {
    if (ended) return;
    if (!result.failureClass) result.failureClass = failureClass;
    if (!result.error) result.error = errorText(error);
    try {
      writeLog({ event: 'failure', failureClass: result.failureClass, error: result.error });
    } catch {
      // 日志流异常不能阻止 result.json 落盘。
    }
    finish(exitCode);
  };
  const send = (message) => {
    if (ended) return;
    writeLog({ direction: 'out', message });
    ws.send(JSON.stringify(message));
  };
  const recordStage = (stage, source) => {
    if (typeof stage !== 'string') return false;
    const last = result.stageTimeline.at(-1);
    if (last?.stage === stage) return false;
    result.stageTimeline.push({ stage, elapsedSec: elapsedSec(), source });
    return true;
  };
  const writeArtifact = (message) => {
    if (message.type !== 'artifact_update') return;
    const version = message.version ?? result.artifacts.length + 1;
    const artifactPath = path.join(outDir, `artifact-v${version}.json`);
    fs.writeFileSync(artifactPath, json(message), 'utf8');
    result.artifacts.push(path.basename(artifactPath));
    latestArtifact = message;
    collectFindings(message, result.validator_findings);
  };
  const sendDraftDecision = (artifact) => {
    const payload = artifact?.draft_candidate;
    const draft = payload?.draft_record;
    const outlineId = draft?.outline_id;
    if (!outlineId || acceptedDrafts.has(outlineId)) return false;
    if (!payload.can_accept || draft.status === 'validation_failed') {
      fail('draft_validation_failed', `Draft ${outlineId} 不可接受，拒绝猜测 rewrite 策略`);
      return true;
    }
    acceptedDrafts.add(outlineId);
    send({ type: 'work_item_draft_decision', outline_id: outlineId, decision: 'accept', feedback: null });
    return true;
  };
  const sendBatchDecision = (artifact) => {
    const batch = artifact?.batch_state;
    if (!batch?.batch_id) return false;
    if (Array.isArray(batch.failure_summary) && batch.failure_summary.length) {
      fail('batch_validation_failed', `Batch ${batch.batch_id} 包含失败 Draft，拒绝猜测 rewrite 策略`);
      return true;
    }
    if (reviewVerdict === 'revise') {
      const action = reviewRepairAction(durableRunHistory, latestReviewComplete);
      if (action.kind === 'fail') {
        fail(action.failureClass, 'durable run_history 不允许继续自动返修');
        return true;
      }
      reviewVerdict = null;
      send({ type: 'work_item_batch_decision', decision: 'rewrite_batch', feedback: FEEDBACK, first_affected_outline_id: null });
      return true;
    }
    send({ type: 'work_item_batch_decision', decision: 'accept_all', feedback: null, first_affected_outline_id: null });
    return true;
  };
  const handleHumanConfirm = () => {
    if (runPolicy !== 'interactive' || !activeNodeId || humanConfirmSent || result.failureClass) return;
    const action = humanConfirmAction({
      activeNodeType,
      reviewVerdict,
      reviewComplete: latestReviewComplete,
      humanGateSnapshot: result.human_gate_snapshot,
      manualRepairsUsed: durableRunHistory?.manual_repairs_used ?? 0,
      manualRepairRequestsSent,
    });
    if (action.kind === 'wait') return;
    if (action.kind === 'fail') {
      fail(action.failureClass, 'interactive human_confirm 的 manual 返修预算已耗尽或节点类型未知');
      return;
    }
    humanConfirmSent = true;
    if (action.kind === 'confirm') {
      send({ type: 'human_confirm', decision: 'confirm', payload: null });
      return;
    }
    manualRepairRequestsSent += 1;
    send(humanConfirmRequestChangeMessage(feedback));
  };
  const handleAuthorConfirm = () => {
    // stage_change 先于节点创建时先等待，避免对尚未知类型的节点猜测决定。
    const action = authorConfirmAction(activeNodeType, Boolean(latestArtifact));
    if (action.kind === 'fail') {
      const detail = action.failureClass === 'unexpected_author_confirm'
        ? `author_confirm 未关联到受支持的 Work Item Plan 节点: ${activeNodeType}`
        : action.failureClass === 'draft_artifact_missing'
          ? 'work_item_draft_confirm 未收到可接受的最新 Draft artifact'
          : action.failureClass === 'batch_artifact_missing'
            ? 'work_item_batch_confirm 未收到可接受的最新 Batch artifact'
            : `不为 ${activeNodeType} 发明自动策略`;
      fail(action.failureClass, detail);
      return;
    }
    if (!activeNodeId || action.kind === 'wait') return;
    if (action.kind === 'select_batch') {
      const selection = generationModeSelectionForNode(activeNodeId, respondedAuthorNodeIds);
      if (selection) send(selection);
      return;
    }
    if (respondedAuthorNodeIds.has(activeNodeId)) return;
    respondedAuthorNodeIds.add(activeNodeId);
    if (action.kind === 'author_accept_with_review') {
      send({ type: 'author_decision', decision: 'accept_with_review' });
      return;
    }
    if (action.kind === 'author_accept') {
      send({ type: 'author_decision', decision: 'accept' });
      return;
    }
    if (action.kind === 'draft_decision') {
      if (!sendDraftDecision(latestArtifact)) {
        fail('draft_artifact_missing', 'work_item_draft_confirm 未收到可接受的最新 Draft artifact');
      }
      return;
    }
    if (action.kind === 'batch_decision' && !sendBatchDecision(latestArtifact)) {
      fail('batch_artifact_missing', 'work_item_batch_confirm 未收到可接受的最新 Batch artifact');
    }
  };
  const applySessionProtocol = (protocol, message) => {
    latestSessionStateMessage = message;
    durableRunHistory = protocol.run_history;
    sessionProtocolSeen = true;
    result.session_status = protocol.session_status;
    result.flow_kind = protocol.flow_kind;
    result.run_policy = protocol.run_policy;
    result.run_history = protocol.run_history;
    result.policy_diagnostics = protocol.policy_diagnostics;
    result.human_gate_snapshot = protocol.human_gate_snapshot;
    result.provider_start_count = protocol.provider_start_count;
  };
  const durableSessionStatePath = () => path.join(
    ARIA_ROOT,
    'projects',
    PROJECT_ID,
    'issues',
    result.issue_id,
    'workspace-sessions',
    `${result.workspace_session_id}.json`,
  );
  const readDurableSessionState = () => {
    const sessionPath = durableSessionStatePath();
    const record = JSON.parse(fs.readFileSync(sessionPath, 'utf8'));
    if (!isRecord(record) || record.id !== result.workspace_session_id) {
      throw new Error(`持久 session 回读不匹配: ${sessionPath}`);
    }
    const message = {
      type: 'session_state',
      session_status: record.status,
      flow_kind: record.flow_kind,
      run_policy: record.run_policy,
      run_history: record.run_history,
      policy_diagnostics: record.policy_diagnostics,
      human_gate_snapshot: record.human_gate_snapshot ?? null,
      provider_start_ledger: record.provider_start_ledger,
    };
    return { message, protocol: sessionStateProtocol(message, runPolicy), sessionPath };
  };
  const finishDurableTerminal = (action, trigger) => {
    const detail = json({
      trigger,
      session_status: result.session_status,
      policy_diagnostics: result.policy_diagnostics,
      human_gate_snapshot: result.human_gate_snapshot,
    });
    writeLog({ event: 'durable_session_terminal', trigger, failureClass: action.failureClass, detail: JSON.parse(detail) });
    result.failureClass = action.failureClass;
    if (action.failureClass !== 'stopped_needs_human') result.error = `SessionState 终态: ${detail}`;
    finish(action.failureClass === 'stopped_needs_human' ? 0 : 1);
  };
  const completeConfirmedSession = () => {
    if (completionVerificationStarted) return;
    completionVerificationStarted = true;
    result.completed = true;
    verifyConfirmedPlan(result, elapsedMs)
      .then(({ plan, workItemIds }) => {
        result.work_item_ids = workItemIds;
        result.validator_findings.push(...(Array.isArray(plan.validator_findings) ? plan.validator_findings : []));
        const handoff = {
          project_id: PROJECT_ID,
          issue_id: result.issue_id,
          plan_id: result.plan_id,
          work_item_ids: workItemIds,
          repository_id: REPOSITORY_ID,
          provider,
          prepare_options: result.prepare_options,
          plan_confirmation_status: plan.status,
          execution_plan_confirm_required: Boolean(plan.options?.require_execution_plan_confirm),
          outDir,
        };
        fs.writeFileSync(path.join(outDir, 'handoff.json'), json(handoff), 'utf8');
        finish(0);
      })
      .catch((error) => fail('handoff_verification_failed', error));
  };
  const finalizeFromDurableSession = (trigger, closeEvent = null) => {
    if (ended || completionVerificationStarted) return true;
    let durable;
    try {
      durable = readDurableSessionState();
    } catch (error) {
      writeLog({ event: 'durable_session_readback_failed', trigger, error: errorText(error) });
      if (trigger === 'ws_close') {
        fail(
          'ws_closed',
          `workspace WebSocket 关闭: code=${closeEvent?.code ?? 'unknown'}; 持久 session 回读失败: ${errorText(error)}`,
        );
        return true;
      }
      return false;
    }
    applySessionProtocol(durable.protocol, durable.message);
    const action = terminalSessionAction(durable.message, awaitingReviewNeedsHumanTerminal, true);
    writeLog({
      event: 'durable_session_readback',
      trigger,
      sessionPath: durable.sessionPath,
      session_status: durable.protocol.session_status,
      action: action.kind,
    });
    if (action.kind === 'complete') {
      completeConfirmedSession();
      return true;
    }
    if (action.kind === 'terminal') {
      finishDurableTerminal(action, trigger);
      return true;
    }
    if (action.kind === 'fail') {
      fail(action.failureClass, `SessionState 失败关闭: ${json({
        session_status: durable.protocol.session_status,
        policy_diagnostics: durable.protocol.policy_diagnostics,
      })}`);
      return true;
    }
    if (trigger === 'ws_close') {
      fail(
        'ws_closed',
        `workspace WebSocket 关闭: code=${closeEvent?.code ?? 'unknown'}; 持久 session 未到终态: ${durable.protocol.session_status}`,
      );
      return true;
    }
    return false;
  };
  const handleStage = (stage, source) => {
    currentStage = stage;
    // 后端可能先广播 stage_change、后创建确认节点；清除旧节点避免误发决策。
    if ((stage === 'author_confirm' || stage === 'human_confirm') && source === 'stage_change') {
      activeNodeId = null;
      activeNodeType = null;
    }
    if (stage !== 'human_confirm') humanConfirmSent = false;
    recordStage(stage, source);
    if (ended) return;
    if (stage === 'completed' && source === 'stage_change') {
      finalizeFromDurableSession('stage_completed');
      return;
    }
    if (!sessionProtocolSeen) return;
    if (stage === 'prepare_context' && !startSent) {
      startSent = true;
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
    } else if (stage === 'author_confirm') {
      handleAuthorConfirm();
    } else if (stage === 'human_confirm') {
      handleHumanConfirm();
    }
  };

  hardTimer = setTimeout(() => {
    fail(result.failureClass ?? 'hard_timeout', result.error ?? `硬超时 ${HARD_LIMIT_MS}ms`);
  }, HARD_LIMIT_MS);

  try {
    if (runPolicy === 'interactive') feedback = readFeedbackFile(feedbackFile);
    let existingSession = null;
    if (existingSessionId) {
      existingSession = findExistingSessionMetadata(existingSessionId);
      if (runPolicy !== existingSession.runPolicy && runPolicy !== 'interactive') {
        writeLog({
          event: 'existing_session_run_policy_override',
          configured_run_policy: runPolicy,
          session_run_policy: existingSession.runPolicy,
        });
      }
      runPolicy = existingSession.runPolicy;
      result.issue_id = existingSession.issueId;
      result.plan_id = existingSession.planId;
      result.workspace_session_id = existingSessionId;
      result.prepare_options = null;
      writeLog({
        event: 'existing_session_selected',
        issue_id: result.issue_id,
        plan_id: result.plan_id,
        session_id: result.workspace_session_id,
        session_path: existingSession.sessionPath,
      });
    }
    if (!existingSession) {
      const issue = await requestJson(
      `${BASE}/api/projects/${encodeURIComponent(PROJECT_ID)}/issues`,
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          title: `WorkItem campaign ${provider} ${rep}`,
          description: description.content,
          repository_id: REPOSITORY_ID,
        }),
        label: 'create issue',
      },
      elapsedMs,
    );
      result.issue_id = extractId(issue, ['issue_id', 'issue.issue_id', 'id']);
      if (!result.issue_id) throw new Error('create issue 响应缺少 issue_id');
      writeLog({ event: 'issue_created', issue_id: result.issue_id });

      const seeded = seedFixtures(result.issue_id);
      writeLog({ event: 'fixtures_seeded', seededAt: seeded.seededAt, destinations: seeded.destinations });

      const prepareOptions = prepareOptionsForProvider(provider, runPolicy);
      const prepared = await requestJson(
        `${BASE}/api/projects/${encodeURIComponent(PROJECT_ID)}/issues/${encodeURIComponent(result.issue_id)}/work-item-plans:prepare`,
        {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ title: `WorkItem campaign ${provider} ${rep}`, ...prepareOptions }),
          label: 'prepare work item plan',
        },
        elapsedMs,
      );
      result.plan_id = extractId(prepared, ['work_item_plan.id', 'work_item_plan.plan_id', 'plan_id']);
      result.workspace_session_id = extractId(prepared, [
        'workspace_session.workspace_session_id',
        'workspace_session.session_id',
        'workspace_session.id',
        'session_id',
      ]);
      if (!result.plan_id || !result.workspace_session_id) {
        throw new Error('prepare 响应缺少 plan_id 或 workspace_session_id');
      }
      result.prepare_options = prepareOptions;
      result.validator_findings.push(...(prepared.work_item_plan?.validator_findings ?? []));
      writeLog({ event: 'plan_prepared', plan_id: result.plan_id, session_id: result.workspace_session_id });
    }

    if (elapsedMs() >= HARD_LIMIT_MS) throw new Error('hard-timeout before WebSocket connection');
    ws = new WebSocket(`${WS_BASE}/api/workspace-sessions/${encodeURIComponent(result.workspace_session_id)}/ws`);
    ws.onopen = () => {
      writeLog({ event: 'ws_open' });
      send({ type: 'hello', session_id: result.workspace_session_id, last_seen_node_id: null });
    };
    ws.onmessage = (event) => {
      try {
        let message;
        try {
          message = JSON.parse(event.data);
        } catch {
          writeLog({ direction: 'in', malformed_raw: String(event.data) });
          fail('malformed_ws_message', `无法解析 WebSocket 消息: ${String(event.data).slice(0, 500)}`);
          return;
        }
        writeLog({ direction: 'in', message });
        if (message.type === 'session_state') {
          const protocol = sessionStateProtocol(message, runPolicy);
          applySessionProtocol(protocol, message);
          const action = terminalSessionAction(message, awaitingReviewNeedsHumanTerminal);
          if (action.kind === 'fail') {
            fail(action.failureClass, `SessionState 失败关闭: ${json({
              session_status: protocol.session_status,
              policy_diagnostics: protocol.policy_diagnostics,
            })}`);
            return;
          }
          if (action.kind === 'terminal') {
            finishDurableTerminal(action, 'session_state');
            return;
          }
          if (action.kind === 'complete') {
            completeConfirmedSession();
            return;
          }
        }
        // completed 是服务端终局通知；即使初始 session_state 丢失，也必须以持久记录为准回读。
        if (message.type === 'stage_change' && message.stage === 'completed') {
          handleStage(message.stage, message.type);
          return;
        }
        if (!sessionProtocolSeen) {
          fail('session_state_missing', `自动处理前必须先收到并校验 session_state，实际为 ${String(message.type)}`);
          return;
        }
        collectUsageByRole(message, usageByRole);
        collectFindings(message, result.validator_findings);
        if (message.stage) recordStage(message.stage, message.type);
        if (ended) return;
        if (Array.isArray(message.timeline_nodes)) {
          message.timeline_nodes.forEach((node) => {
            if (node?.node_id) nodesById.set(node.node_id, node);
          });
        }
        if (message.type === 'timeline_node_created' && message.node?.node_id) {
          nodesById.set(message.node.node_id, message.node);
        }
        if (message.active_node_id) {
          activeNodeId = message.active_node_id;
          activeNodeType = nodesById.get(activeNodeId)?.node_type ?? activeNodeType;
        }
        switch (message.type) {
        case 'session_state':
        case 'stage_change':
          handleStage(message.stage, message.type);
          break;
        case 'artifact_update':
          writeArtifact(message);
          // 只落盘；Draft/Batch 决策必须等对应确认节点成为 active 后再发送。
          break;
        case 'provider_select_request': {
          const selection = providerRolesForSelection(message, currentStage);
          result.provider_selections.push({
            elapsedSec: elapsedSec(),
            stage: message.stage ?? currentStage ?? null,
            defaults: message.defaults ?? null,
            roles: selection.roles,
            strategy: selection.strategy,
            assumption: selection.assumption,
          });
          selection.roles.forEach((role) => send({ type: 'provider_select', role, provider }));
          break;
        }
        case 'permission_request':
          result.permission_approvals += 1;
          send({ type: 'permission_response', id: message.id, approved: true, reason: null });
          break;
        case 'choice_request': {
          const choice = firstChoice(message);
          if (!choice.ids.length) {
            fail('choice_without_options', `choice_request ${message.id ?? '<missing>'} 没有可选项`);
            break;
          }
          result.choices.push({ id: message.id, elapsedSec: elapsedSec(), selected_option_ids: choice.ids, strategy: choice.strategy });
          send({ type: 'choice_response', id: message.id, selected_option_ids: choice.ids, free_text: null, answers: choice.answers });
          break;
        }
        case 'review_complete': {
          reviewVerdict = message.verdict;
          latestReviewComplete = message;
          const findings = message.findings ?? [];
          result.verdicts.push({ verdict: message.verdict, round: message.round ?? null, elapsedSec: elapsedSec(), findings });
          if (Array.isArray(findings)) result.validator_findings.push(...findings);
          const action = reviewCompleteAction(reviewVerdict, runPolicy);
          if (action.kind === 'human_confirm_request_change') {
            // review_complete 可能先于 human_confirm timeline node 广播；此处只记录
            // 非终态事实，等 node 成为 active 后由 handleHumanConfirm 发送。
            awaitingReviewNeedsHumanTerminal = false;
            if (currentStage === 'human_confirm' && activeNodeType === 'human_confirm') {
              handleHumanConfirm();
            }
          } else if (action.kind === 'await_terminal_session_state') {
            awaitingReviewNeedsHumanTerminal = true;
            const terminal = terminalSessionAction(latestSessionStateMessage, true);
            if (terminal.kind === 'fail') {
              fail(terminal.failureClass, `SessionState 失败关闭: ${json({
                session_status: result.session_status,
                policy_diagnostics: result.policy_diagnostics,
              })}`);
              break;
            }
            writeLog({
              event: 'review_needs_human_waiting_for_session_terminal',
              verdict: reviewVerdict,
            });
          }
          break;
        }
        case 'review_decision_required': {
          const action = reviewDecisionAction(message, reviewVerdict, latestReviewComplete);
          const audit = {
            node_id: message.node_id ?? null,
            round: message.round ?? null,
            elapsedSec: elapsedSec(),
            verdict: reviewVerdict,
            options: message.options ?? null,
            selected_option: action.decision ?? null,
            strategy: action.strategy,
          };
          result.review_decisions.push(audit);
          if (action.kind === 'respond') {
            send({
              type: 'review_decision_response',
              decision: action.decision,
              extra_context: action.extra_context,
            });
            break;
          }
          if (action.kind === 'revise') {
            const repairAction = reviewRepairAction(durableRunHistory, latestReviewComplete);
            if (repairAction.kind === 'fail') {
              fail(repairAction.failureClass, 'durable run_history 不允许继续自动返修');
            } else {
              reviewVerdict = null;
              send({ type: 'select_revision_path', path: action.decision, extra_context: FEEDBACK });
            }
            break;
          }
          const detail = Array.isArray(message.options) && message.options.length
            ? `未提供“跳过可选建议”选项: ${JSON.stringify(message.options)}`
            : `缺少 options，且 verdict=${reviewVerdict ?? 'unknown'} 不允许猜测响应`;
          fail('review_decision_unhandled', `review_decision_required ${detail}`);
          break;
        }
        case 'timeline_node_created':
          if (message.node?.status === 'active') {
            activeNodeId = message.node.node_id ?? activeNodeId;
            activeNodeType = message.node.node_type ?? activeNodeType;
            if (currentStage === 'author_confirm') handleAuthorConfirm();
            if (currentStage === 'human_confirm') handleHumanConfirm();
          }
          break;
        case 'provider_status':
        case 'execution_event':
          break;
        case 'timeline_node_updated':
        case 'stream_chunk':
        case 'message_complete':
        case 'provider_locked':
        case 'pong':
        case 'human_presentation_revision_saved':
        case 'human_presentation_revision_save_failed':
        case 'linked_workspace_amendment_created':
          break;
        case 'error':
          fail('workspace_error', message.message ?? 'workspace error');
          break;
        case 'protocol_error':
          fail('protocol_error', `${message.code ?? 'protocol_error'}: ${message.message ?? ''}`);
          break;
        default:
          fail('unknown_ws_message', `未知 workspace WebSocket 消息类型: ${String(message.type)}`);
        }
      } catch (error) {
        fail('driver_error', error);
      }
    };
    ws.onerror = (event) => {
      if (ended) return;
      writeLog({ event: 'ws_error', message: event?.message ?? 'workspace WebSocket error' });
      if (!finalizeFromDurableSession('ws_error')) {
        fail('ws_transport_error', event?.message ?? 'workspace WebSocket error');
      }
    };
    ws.onclose = (event) => {
      if (ended) return;
      writeLog({ event: 'ws_close', code: event?.code, reason: event?.reason, wasClean: event?.wasClean });
      finalizeFromDurableSession('ws_close', event);
    };
  } catch (error) {
    const message = errorText(error);
    fail(/timeout/i.test(message) ? 'hard_timeout' : 'setup_error', message);
  }
}

async function main() {
  const options = parseArgs(process.argv);
  let fixtureDigests;
  let description;
  if (!options.existingSessionId) {
    try {
      fixtureDigests = validateFixtures();
      description = loadDescription();
    } catch (error) {
      console.error(`启动校验失败: ${errorText(error)}`);
      process.exit(2);
    }
  } else {
    // existing session 已经完成 issue/fixture/prepare；续跑只需要 session ID。
    fixtureDigests = {};
    description = { content: '', digest: null };
  }
  if (options.dryRun) {
    console.log(json({
      dry_run: true,
      provider: options.provider,
      repetition: Number(options.rep),
      outDir: path.join(options.outRoot, options.provider, `rep${options.rep}`),
      project_id: PROJECT_ID,
      repository_id: REPOSITORY_ID,
      aria_data_root: ARIA_ROOT,
      hard_timeout_ms: HARD_LIMIT_MS,
      fixture_digests: fixtureDigests,
      description_digest: description.digest,
      prepare_options: options.existingSessionId
        ? null
        : prepareOptionsForProvider(options.provider, options.runPolicy),
      no_http_or_websocket_requests: true,
    }));
    return;
  }
  await runCampaign({ ...options, fixtureDigests, description });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}

export {
  authorConfirmAction,
  collectUsageByRole,
  generationModeSelectionForNode,
  hasMustFixFindings,
  humanConfirmAction,
  humanConfirmRequestChangeMessage,
  providerRolesForSelection,
  prepareOptionsForProvider,
  reviewCompleteAction,
  reviewCycleId,
  reviewDecisionAction,
  reviewRepairAction,
  sessionStateProtocol,
  skipOptionalFindingsOption,
  terminalSessionAction,
};
