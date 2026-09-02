#!/usr/bin/env node
/**
 * Work Item Plan campaign 单样本驱动器。
 * 仅在显式省略 --dry-run 时才会调用 Aria HTTP/WS 与 provider。
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { createInterface } from 'node:readline/promises';
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
// 服务端 durable 数据根固定为当前 workspace 的 .aria；campaign 环境变量不得重定向读路径。
const ARIA_ROOT = path.join(REPO_ROOT, '.aria');
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

function normalizedProviderStartLedger(value, required = false) {
  if (value === undefined) {
    if (required) throw new Error('$.provider_start_ledger 必须是数组');
    return [];
  }
  if (!Array.isArray(value)) throw new Error('$.provider_start_ledger 必须是数组');
  const entriesByKey = new Map();
  value.forEach((entry, index) => {
    if (!isRecord(entry) || typeof entry.provider_start_idempotency_key !== 'string' || !entry.provider_start_idempotency_key) {
      throw new Error(`$.provider_start_ledger[${index}].provider_start_idempotency_key 必须是非空字符串`);
    }
    if (typeof entry.started !== 'boolean') {
      throw new Error(`$.provider_start_ledger[${index}].started 必须是布尔值`);
    }
    const key = entry.provider_start_idempotency_key;
    const prior = entriesByKey.get(key);
    // 恢复重放的同 key 账本条目合并为一个，并保留“任一次已启动”的不可逆事实。
    entriesByKey.set(key, {
      provider_start_idempotency_key: key,
      started: Boolean(prior?.started || entry.started),
    });
  });
  return [...entriesByKey.values()];
}

function providerStartCount(value, required = false) {
  return normalizedProviderStartLedger(value, required)
    .filter((entry) => entry.started)
    .length;
}

// 仅从 SessionState 的 durable 字段读取策略、计数和 provider 启动账本；不使用 stage/event 推断。
function sessionStateProtocol(
  message,
  expectedRunPolicy = CONFIGURED_RUN_POLICY,
  expectedFlowKind = EXPECTED_FLOW_KIND,
) {
  if (!isRecord(message) || message.type !== 'session_state') {
    throw new Error('首条状态消息必须是 session_state');
  }
  if (message.flow_kind !== expectedFlowKind) {
    throw new Error(`$.flow_kind 必须为 ${expectedFlowKind}，实际为 ${String(message.flow_kind)}`);
  }
  if (message.run_policy !== expectedRunPolicy) {
    throw new Error(`$.run_policy 必须为 ${expectedRunPolicy}，实际为 ${String(message.run_policy)}`);
  }
  if (typeof message.session_status !== 'string' || !message.session_status) {
    throw new Error('$.session_status 必须是非空字符串');
  }
  const providerStartLedger = normalizedProviderStartLedger(
    message.provider_start_ledger,
    expectedFlowKind === 'single_candidate',
  );
  return {
    session_status: message.session_status,
    flow_kind: message.flow_kind,
    run_policy: message.run_policy,
    run_history: normalizedRunHistory(message.run_history),
    policy_diagnostics: normalizedPolicyDiagnostics(message.policy_diagnostics),
    human_gate_snapshot: message.human_gate_snapshot ?? null,
    provider_start_ledger: providerStartLedger,
    provider_start_count: providerStartLedger.filter((entry) => entry.started).length,
  };
}

// `terminal` 表示服务端已持久化的合法终局，driver 应写结果并结束，不能再等待 WS 关闭。
function terminalSessionAction(
  message,
  awaitingReviewNeedsHumanTerminal = false,
  terminalReadback = false,
  flowKind = EXPECTED_FLOW_KIND,
) {
  const diagnostics = normalizedPolicyDiagnostics(message?.policy_diagnostics);
  if (message?.session_status === 'confirmed' || (
    flowKind === 'single_candidate' && message?.session_status === 'completed'
  )) return { kind: 'complete' };
  if (message?.session_status === 'stopped_needs_human') {
    return { kind: 'terminal', failureClass: 'stopped_needs_human' };
  }
  if (message?.session_status === 'failed') {
    return { kind: 'terminal', failureClass: 'policy_failed' };
  }
  if (
    flowKind !== 'single_candidate'
    && message?.session_status === 'waiting_for_human'
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

// 阶段 3 语法：action := "request-change:" feedback | "confirm" | "abandon" | "advance"。
// 旧 request-change/confirm 拼写逐字保留（阶段 2 脚本兼容）；abandon 映射 terminate，
// advance 只在 durable Confirmed 回读后作为独立 typed command 发送。
export const STAGE3_HUMAN_SCRIPT_GRAMMAR = {
  version: 3,
  actions: ['request-change:<反馈文本>', 'confirm', 'abandon', 'advance'],
};

const STAGE3_COMMAND_KINDS = new Set(['human_gate_feedback', 'advance']);

function parseHumanScript(script) {
  if (script === undefined || script === null) return [];
  if (typeof script !== 'string') throw new Error('ARIA_HUMAN_SCRIPT 必须是字符串');
  if (script.trim() === '') return [];
  return script
    .split(';')
    .map((rawEntry) => rawEntry.trim())
    .filter(Boolean)
    .map((entry, index) => {
      const separator = entry.indexOf(':');
      const decision = (separator === -1 ? entry : entry.slice(0, separator)).trim();
      if (decision === 'confirm') {
        if (separator !== -1) throw new Error(`ARIA_HUMAN_SCRIPT 第 ${index + 1} 条 confirm 不接受描述`);
        return { decision: 'confirm', description: null };
      }
      if (decision === 'abandon') {
        if (separator !== -1) throw new Error(`ARIA_HUMAN_SCRIPT 第 ${index + 1} 条 abandon 不接受描述`);
        return { decision: 'abandon', description: null };
      }
      if (decision === 'advance') {
        if (separator !== -1) throw new Error(`ARIA_HUMAN_SCRIPT 第 ${index + 1} 条 advance 不接受描述`);
        return { decision: 'advance', description: null };
      }
      if (decision === 'request-change') {
        const description = separator === -1 ? '' : entry.slice(separator + 1).trim();
        if (!description) {
          throw new Error(`ARIA_HUMAN_SCRIPT 第 ${index + 1} 条 request-change 必须提供冒号后的反馈文本`);
        }
        return { decision: 'request-change', description };
      }
      throw new Error(`ARIA_HUMAN_SCRIPT 第 ${index + 1} 条仅支持 confirm、request-change:<文本>、abandon 或 advance`);
    });
}

function humanConfirmScriptAction(script, gateIndex, runPolicy) {
  if (runPolicy !== 'interactive') return null;
  const entry = script[gateIndex];
  if (entry) return { ...entry, source: 'human_script' };
  return { decision: 'confirm', description: null, source: 'human_script_exhausted_default' };
}

function humanConfirmMessage(action) {
  if (action.decision === 'confirm') {
    return { type: 'human_confirm', decision: 'confirm', payload: null };
  }
  if (action.decision === 'request-change' && typeof action.description === 'string') {
    return humanConfirmRequestChangeMessage(action.description);
  }
  throw new Error(`不支持的人工确认决策: ${String(action.decision)}`);
}

// —— 阶段 3 typed 人工门与 advance 模拟（task 8.1）——

// 仅 flow_kind=single_candidate && run_policy=interactive 切换到 typed 编码；其余 flow 维持阶段 2 行为。
function stage3TypedFlowActive(flowKind, runPolicy) {
  return flowKind === 'single_candidate' && runPolicy === 'interactive';
}

// 客户端幂等键：由稳定 run 身份（provider/rep/issue-or-existing-session）+ 脚本序号 + kind
// 确定性生成；重连/重复发送/进程重启（同 checkpoint 或同身份）都复用原值。
function campaignCommandId({ campaignRunId, actionIndex, kind }) {
  if (typeof campaignRunId !== 'string' || !campaignRunId.trim()) {
    throw new Error('campaignCommandId 要求非空 campaignRunId');
  }
  if (!Number.isInteger(actionIndex) || actionIndex < 0) {
    throw new Error('campaignCommandId 要求非负整数 actionIndex');
  }
  if (!STAGE3_COMMAND_KINDS.has(kind)) {
    throw new Error(`campaignCommandId kind 必须是 ${[...STAGE3_COMMAND_KINDS].join(' 或 ')}`);
  }
  const digest = sha256(`${campaignRunId}\n${kind}\n${String(actionIndex)}`).slice(0, 10);
  return `cmd-${kind}-${actionIndex}-${digest}`;
}

// SC typed flow 的 wire 映射：request-change 绝不再编码 HumanConfirmDecision::RequestChange。
function stage3HumanMessage(action, context) {
  const commandId = context?.commandId ?? null;
  if (action?.decision === 'request-change') {
    if (!commandId) throw new Error('request-change 的 typed 编码必须携带 commandId');
    return { type: 'human_gate_feedback', command_id: commandId, feedback: action.description };
  }
  if (action?.decision === 'confirm') {
    // REQ-CG-02：SC HumanConfirm stage 的服务端准入表只收 HumanGateFeedback|Confirm|HumanConfirm{Terminate}；
    // confirm 编码为裸 typed 消息（WsInMessage::Confirm，snake_case tag），不得发 human_confirm{decision:"confirm"}。
    // （台账 Ruling：契约效力高于 brief 原文；legacy 分支的 human_confirm 原样不动。）
    return { type: 'confirm' };
  }
  if (action?.decision === 'abandon') {
    return { type: 'human_confirm', decision: 'terminate', payload: null };
  }
  if (action?.decision === 'advance') {
    if (!commandId) throw new Error('advance 的 typed 编码必须携带 commandId');
    return { type: 'advance', command_id: commandId };
  }
  throw new Error(`不支持的人工脚本动作: ${String(action?.decision)}`);
}

function stage3FeedbackDigest(feedback) {
  return sha256(String(feedback ?? ''));
}

// ws.jsonl 出站脱敏：typed feedback 只留 digest/长度/command_id，不写反馈全文。
function stage3OutboundLogEntry(message) {
  if (message?.type === 'human_gate_feedback') {
    return {
      type: message.type,
      command_id: message.command_id,
      feedback_digest: stage3FeedbackDigest(message.feedback),
      feedback_length: typeof message.feedback === 'string' ? message.feedback.length : 0,
    };
  }
  return message;
}

// 事件驱动消费判定：只有服务端事件/durable replay 证明“服务端已接受该动作”才消费。
// busy/rejected/failed 均不消费；duplicate 由调用侧状态机去重。
function shouldConsumeHumanAction(message, durableState, action) {
  if (!isRecord(message) || !isRecord(action)) return false;
  if (action.decision === 'request-change') {
    const commandId = action.commandId;
    if (message.type === 'human_gate_turn_open' && message.command_id === commandId) return true;
    if (Array.isArray(durableState?.replayedCommandIds) && durableState.replayedCommandIds.includes(commandId)) {
      return true;
    }
    return false;
  }
  if (action.decision === 'confirm') {
    if (message.type === 'human_gate_closed' && ['confirm', 'approve'].includes(String(message.decision))) return true;
    if (durableState?.confirmedPlan === true) return true;
    return false;
  }
  if (action.decision === 'abandon') {
    return message.type === 'human_gate_closed' && String(message.decision) === 'terminate';
  }
  if (action.decision === 'advance') {
    const commandId = action.commandId;
    if (message.type === 'advance_completed' && message.command_id === commandId) return true;
    if (Array.isArray(durableState?.advanceRecords)
      && durableState.advanceRecords.some((record) => record?.command_id === commandId)) {
      return true;
    }
    return false;
  }
  return false;
}

// advance 模拟决策：已有 durable advance 记录（幂等命中）优先；否则必须 durable
// Confirmed 回读成功且此前未 advance 才允许发送 typed command。
function advanceSimulationAction({ confirmedPlan, existingAdvance, action }) {
  if (existingAdvance) return 'complete';
  if (!action || action.decision !== 'advance') return 'wait';
  if (!confirmedPlan) return 'wait';
  return 'send';
}

// advance 等待期的收尾决策（task 8.1 修复轮 M2）：rejected 不消费脚本动作，但对 rejected
// 无条件清 advanceFinishPending 并按既定策略收尾（plan 已 Confirmed，不悬挂至 hard timeout；
// 语义对齐断线分支 stage3_advance_outcome_unknown）；completed 仍要求动作确认消费后才收尾。
// 返回收尾日志事件（writeLog 条目）或 null（继续等待）。
function stage3AdvanceFinishPlan({ finishPending, message, consumedKinds }) {
  if (finishPending !== true) return null;
  if (message?.type === 'advance_rejected') {
    return {
      event: 'stage3_advance_rejected',
      command_id: message.command_id ?? null,
      code: message.code ?? null,
    };
  }
  if (message?.type === 'advance_completed' && (consumedKinds ?? []).includes('advance')) {
    return {
      event: 'stage3_advance_simulation',
      decision: message.type,
      command_id: message.command_id ?? null,
      attempt_id: message.attempt_id ?? null,
      code: message.code ?? null,
    };
  }
  return null;
}

// 阶段 3 门控制器：command/turn 级状态机（open→running→completed/failed→下一脚本动作）。
// 脱账/审计字段只含 digest/长度/command_id/turn_id；脚本耗尽仅在门 Waiting 且无 in-flight
// 时沿用 human_script_exhausted_default。
function createStage3GateController({
  actions,
  campaignRunId,
  checkpoint = null,
  persistCheckpoint = null,
}) {
  const script = Array.isArray(actions) ? actions : [];
  const commandIds = {
    ...(isRecord(checkpoint?.command_ids) ? checkpoint.command_ids : {}),
  };
  const checkpointState = { campaign_run_id: campaignRunId, command_ids: commandIds };
  const syntheticActions = [];
  const turns = [];
  const advanceActions = [];
  const recoveryChecks = [];
  const ledger = { before: null, after: null };
  let actionIndex = 0;
  let awaitingAccept = null;
  let inFlightTurn = null;
  let exhaustedEpisodeOpen = false;

  const actionAt = (index) => script[index] ?? syntheticActions[index - script.length] ?? null;
  const keyFor = (index, kind) => `${kind}#${index}`;
  const resolveCommandId = (index, kind) => {
    const key = keyFor(index, kind);
    if (typeof commandIds[key] === 'string') return commandIds[key];
    return campaignCommandId({ campaignRunId, actionIndex: index, kind });
  };
  const materializeCommandId = (index, kind) => {
    const key = keyFor(index, kind);
    if (typeof commandIds[key] === 'string') {
      recoveryChecks.push({
        check: 'checkpoint_command_id_reused',
        key,
        command_id: commandIds[key],
        actionIndex: index,
        kind,
      });
      return commandIds[key];
    }
    commandIds[key] = campaignCommandId({ campaignRunId, actionIndex: index, kind });
    if (persistCheckpoint) persistCheckpoint(structuredClone(checkpointState));
    return commandIds[key];
  };
  const currentWithCommand = () => {
    const action = actionAt(actionIndex);
    if (!action) return null;
    if (action.decision === 'request-change') {
      return { ...action, commandId: resolveCommandId(actionIndex, 'human_gate_feedback') };
    }
    if (action.decision === 'advance') {
      return { ...action, commandId: resolveCommandId(actionIndex, 'advance') };
    }
    return { ...action, commandId: null };
  };
  const turnRecordFor = (action, commandId) => {
    const existing = turns.find((turn) => turn.command_id === commandId);
    if (existing) return existing;
    const record = {
      action_index: actionIndex,
      command_id: commandId,
      feedback_digest: stage3FeedbackDigest(action.description),
      feedback_length: typeof action.description === 'string' ? action.description.length : 0,
      turn_id: null,
      remaining_budget: null,
      status: 'sent',
      busy_events: 0,
    };
    turns.push(record);
    return record;
  };
  const commandKindForAction = (action) => {
    if (!action) return null;
    if (action.decision === 'request-change') return 'human_gate_feedback';
    return action.decision;
  };
  const consumeCurrent = (consumed, via) => {
    const action = actionAt(actionIndex);
    consumed.push({ actionIndex, kind: commandKindForAction(action), via });
    actionIndex += 1;
    awaitingAccept = null;
    exhaustedEpisodeOpen = false;
    return action;
  };

  const onGateWaiting = ({ reconnect = false } = {}) => {
    if (inFlightTurn) return null;
    const action = actionAt(actionIndex);
    if (action?.decision === 'advance') return null;
    if (!action) {
      if (exhaustedEpisodeOpen) return null;
      exhaustedEpisodeOpen = true;
      return {
        message: stage3HumanMessage({ decision: 'confirm' }, { commandId: null }),
        actionIndex: null,
        commandId: null,
        source: 'human_script_exhausted_default',
      };
    }
    if (awaitingAccept && !reconnect) return null;
    const commandId = action.decision === 'request-change'
      ? materializeCommandId(actionIndex, 'human_gate_feedback')
      : null;
    const message = stage3HumanMessage(action, { commandId });
    awaitingAccept = { actionIndex, kind: action.decision, commandId };
    if (action.decision === 'request-change') turnRecordFor(action, commandId);
    return { message, actionIndex, commandId, source: 'human_script' };
  };

  const onInbound = (message) => {
    const consumed = [];
    const current = currentWithCommand();
    const openTurnFor = (commandId) => turns.find((turn) => turn.command_id === commandId && turn.status !== 'completed' && turn.status !== 'failed');
    const terminalFor = (turnId) => turns.find((turn) => (turn.turn_id === turnId || turn.turn_id === null)
      && (turn.status === 'sent' || turn.status === 'open'));

    if (message?.type === 'human_gate_turn_open') {
      if (current?.decision === 'request-change' && shouldConsumeHumanAction(message, {}, current)) {
        const record = turnRecordFor(current, current.commandId);
        record.turn_id = message.turn_id;
        record.remaining_budget = message.remaining_budget;
        record.status = 'open';
        inFlightTurn = { turnId: message.turn_id, commandId: message.command_id };
        consumeCurrent(consumed, 'human_gate_turn_open');
      } else {
        const replayed = turns.find((turn) => turn.command_id === message.command_id);
        if (replayed) {
          replayed.turn_id ??= message.turn_id;
          replayed.remaining_budget ??= message.remaining_budget;
          recoveryChecks.push({
            check: 'duplicate_turn_open_replayed',
            command_id: message.command_id,
            turn_id: message.turn_id,
          });
        }
      }
      return { consumed, outbound: null };
    }
    if (message?.type === 'human_gate_turn_completed' || message?.type === 'human_gate_turn_failed') {
      const record = terminalFor(message.turn_id);
      if (record) {
        record.status = message.type === 'human_gate_turn_completed' ? 'completed' : 'failed';
        if (message.type === 'human_gate_turn_completed') record.artifact_ref = message.artifact_ref;
        else record.failure_class = message.failure_class;
        if (inFlightTurn && (inFlightTurn.turnId === message.turn_id || inFlightTurn.turnId === null)) {
          inFlightTurn = null;
        }
        return { consumed, outbound: onGateWaiting() };
      }
      return { consumed, outbound: null };
    }
    if (message?.type === 'human_gate_busy') {
      const record = terminalFor(message.turn_id);
      if (record) record.busy_events += 1;
      return { consumed, outbound: null };
    }
    if (message?.type === 'human_gate_closed') {
      exhaustedEpisodeOpen = false;
      if (current && shouldConsumeHumanAction(message, {}, current)) {
        consumeCurrent(consumed, 'human_gate_closed');
      } else if (awaitingAccept?.kind === 'confirm' || awaitingAccept?.kind === 'abandon') {
        awaitingAccept = null;
      }
      return { consumed, outbound: null };
    }
    if (message?.type === 'advance_completed' || message?.type === 'advance_rejected') {
      if (current?.decision === 'advance') {
        const record = advanceActions.find((entry) => entry.command_id === current.commandId)
          ?? {
            action_index: actionIndex,
            command_id: current.commandId,
            decision: 'send',
            status: 'sent',
            attempt_id: null,
            workspace_entry: null,
          };
        if (shouldConsumeHumanAction(message, {}, current)) {
          if (!advanceActions.includes(record)) advanceActions.push(record);
          record.status = 'completed';
          record.attempt_id = message.attempt_id;
          record.workspace_entry = message.workspace_entry;
          consumeCurrent(consumed, message.type);
        } else if (message.type === 'advance_rejected') {
          // rejected：记录 advance_actions[].status='rejected' 审计，但不消费脚本动作
          // （只有 completed/同 record replay 才消费）；收尾由 driver 的 stage3AdvanceFinishPlan
          // 对 rejected 无条件执行，不再悬挂至 hard timeout。
          if (!advanceActions.includes(record)) advanceActions.push(record);
          record.status = 'rejected';
          record.code = message.code ?? null;
        }
      }
      return { consumed, outbound: null };
    }
    return { consumed, outbound: null };
  };

  const onDurableState = ({ replayedCommandIds = [], advanceRecords = [], confirmedPlan = false } = {}) => {
    const consumed = [];
    const current = currentWithCommand();
    if (current?.decision === 'request-change'
      && replayedCommandIds.includes(current.commandId)) {
      const record = turnRecordFor(current, current.commandId);
      record.status = 'open';
      inFlightTurn = { turnId: null, commandId: current.commandId };
      recoveryChecks.push({
        check: 'durable_command_replay_consumed',
        command_id: current.commandId,
        actionIndex,
      });
      consumeCurrent(consumed, 'durable_replay');
    } else if (current?.decision === 'advance'
      && advanceRecords.some((entry) => entry?.command_id === current.commandId)) {
      const replayed = advanceRecords.find((entry) => entry?.command_id === current.commandId);
      advanceActions.push({
        action_index: actionIndex,
        command_id: current.commandId,
        decision: 'complete',
        status: 'completed',
        attempt_id: replayed?.attempt_id ?? null,
        workspace_entry: null,
      });
      recoveryChecks.push({
        check: 'durable_advance_record_replayed',
        command_id: current.commandId,
        actionIndex,
      });
      consumeCurrent(consumed, 'durable_replay');
    } else if (current?.decision === 'confirm' && confirmedPlan === true) {
      consumeCurrent(consumed, 'durable_confirmed_readback');
    }
    return { consumed, outbound: null };
  };

  const onConfirmedDurable = ({ providerStartLedger = [], existingAdvance = null } = {}) => {
    const action = actionAt(actionIndex);
    if (action?.decision !== 'advance') return { decision: null, commandId: null, outbound: null };
    const decision = advanceSimulationAction({ confirmedPlan: true, existingAdvance, action });
    if (decision === 'complete') {
      ledger.before = providerStartLedger;
      const commandId = resolveCommandId(actionIndex, 'advance');
      advanceActions.push({
        action_index: actionIndex,
        command_id: commandId,
        decision: 'complete',
        status: 'completed',
        attempt_id: existingAdvance?.attempt_id ?? null,
        workspace_entry: null,
      });
      ledger.after = providerStartLedger;
      actionIndex += 1;
      return { decision: 'complete', commandId, outbound: null };
    }
    if (decision !== 'send') return { decision: 'wait', commandId: null, outbound: null };
    ledger.before = providerStartLedger;
    const commandId = materializeCommandId(actionIndex, 'advance');
    const message = stage3HumanMessage(action, { commandId });
    advanceActions.push({
      action_index: actionIndex,
      command_id: commandId,
      decision: 'send',
      status: 'sent',
      attempt_id: null,
      workspace_entry: null,
    });
    return { decision: 'send', commandId, outbound: message };
  };

  return {
    commandIdFor: (index, kind) => resolveCommandId(index, kind),
    currentAction: () => actionAt(actionIndex),
    enqueueAction: (action) => {
      syntheticActions.push({ ...action });
      return script.length + syntheticActions.length - 1;
    },
    onGateWaiting,
    onInbound,
    onDurableState,
    onConfirmedDurable,
    noteProviderLedgerAfter: (ledgerAfter) => {
      ledger.after = Array.isArray(ledgerAfter) ? ledgerAfter : [];
    },
    resultFields: () => ({
      human_gate_turns: structuredClone(turns),
      advance_actions: structuredClone(advanceActions),
      takeover_actions: [],
      durable_recovery_checks: structuredClone(recoveryChecks),
      provider_start_ledger_before_after: structuredClone(ledger),
    }),
  };
}

function isHumanGateStage(stage) {
  return stage === 'human_confirm' || stage === 'waiting_for_human';
}

function shouldClearPendingGateNode(
  stage,
  source,
  stageActiveNodeId = null,
  currentActiveNodeType = null,
) {
  return source === 'stage_change'
    && (stage === 'author_confirm' || isHumanGateStage(stage))
    && !stageActiveNodeId
    && (!isHumanGateStage(stage) || currentActiveNodeType !== 'human_confirm');
}

function activeNodeTypeForActiveNode(nodesById, activeNodeId, previousActiveNodeId, previousActiveNodeType) {
  if (!activeNodeId) return null;
  const nodeType = nodesById.get(activeNodeId)?.node_type;
  if (nodeType) return nodeType;
  return activeNodeId === previousActiveNodeId ? previousActiveNodeType : null;
}

function singleCandidateOutboundAllowed(message, runPolicy = CONFIGURED_RUN_POLICY) {
  // 阶段 3：interactive 下增加 typed human_gate_feedback/advance/裸 confirm（REQ-CG-02 SC 准入表）；
  // legacy SC 决策族依旧拒绝。
  return message?.type === 'start_generation' || (
    runPolicy === 'interactive'
    && ['human_confirm', 'human_gate_feedback', 'advance', 'confirm'].includes(message?.type)
  );
}

function humanGateActionAudit({
  sequence,
  enteredAt,
  elapsedSec,
  stage,
  reviewVerdict,
  activeNodeId,
  response = null,
  message = null,
  sentAt = null,
}) {
  return {
    sequence,
    enteredAt,
    elapsedSec,
    stage,
    reviewVerdict,
    active_node_id: activeNodeId,
    source: response?.source ?? null,
    message: message
      ? {
        decision: message.decision,
        description_summary: humanDescriptionSummary(message.payload?.description),
      }
      : null,
    ...(sentAt ? { sentAt } : {}),
  };
}

function parseHumanStdinCommand(line) {
  const command = String(line ?? '').trim();
  if (command === 'confirm') return { decision: 'confirm', description: null, source: 'human_stdin' };
  const requestChange = /^request-change\s+(.+)$/u.exec(command);
  if (requestChange?.[1]?.trim()) {
    return {
      decision: 'request-change',
      description: requestChange[1].trim(),
      source: 'human_stdin',
    };
  }
  throw new Error('stdin 人工确认仅接受 confirm 或 request-change <文本>');
}

async function readHumanStdinAction({ sequence, stage, reviewVerdict }) {
  console.log(
    `[ARIA_HUMAN_MODE=stdin] 人工门 #${sequence}：stage=${stage ?? 'unknown'}；review verdict=${reviewVerdict ?? 'unknown'}。`,
  );
  console.log('可选命令：confirm | request-change <文本>');
  const readline = createInterface({ input: process.stdin, output: process.stdout });
  try {
    return parseHumanStdinCommand(await readline.question('human> '));
  } finally {
    readline.close();
  }
}

function humanDescriptionSummary(description) {
  if (typeof description !== 'string') return null;
  const compact = description.replace(/\s+/gu, ' ').trim();
  return compact.length <= 240 ? compact : `${compact.slice(0, 237)}...`;
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

function legacySingleCandidateDecisionMessage(message, runPolicy = CONFIGURED_RUN_POLICY) {
  if (!isRecord(message)) return null;
  const interactiveHumanGate = runPolicy === 'interactive' && (
    (message.type === 'stage_change' && isHumanGateStage(message.stage))
    || (message.type === 'timeline_node_created' && message.node?.node_type === 'human_confirm')
  );
  if (interactiveHumanGate) return null;
  if (new Set([
    'provider_select_request',
    'choice_request',
    'review_decision_required',
  ]).has(message.type)) return message;
  if (message.type === 'stage_change' && new Set([
    'author_confirm',
    'human_confirm',
    'review_decision',
  ]).has(message.stage)) return message;
  if (message.type === 'timeline_node_created' && (
    message.node?.node_type === 'human_confirm'
    || AUTHOR_CONFIRM_NODE_TYPES.has(message.node?.node_type)
  )) {
    return message;
  }
  return null;
}

function confirmedCountForPlanStatus(status) {
  return typeof status === 'string' && status.toLowerCase() === 'confirmed' ? 1 : 0;
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

function applyResultTiming(result, durationMs) {
  result.finishedAt = now();
  result.duration_ms = durationMs;
  result.elapsedSec = Number((durationMs / 1_000).toFixed(3));
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
    ...(runPolicy === 'interactive' ? { humanGateActions: [] } : {}),
    // 阶段 3 typed 审计面：仅 interactive 预留；auto 策略保持阶段 2 模板不变。
    ...(runPolicy === 'interactive' ? {
      human_gate_turns: [],
      advance_actions: [],
      takeover_actions: [],
      durable_recovery_checks: [],
      provider_start_ledger_before_after: null,
    } : {}),
    provider_start_count: 0,
    provider_start_ledger: [],
    legacy_decision_messages: [],
    confirmed_count: 0,
    duration_ms: null,
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
  let feedback = FEEDBACK;
  let humanScript = [];
  let humanScriptConfigured = false;
  let humanMode = null;
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
  const humanConfirmSentNodeIds = new Set();
  let humanGateEnteredAt = null;
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
  // —— 阶段 3 typed 门/advance 控制器（仅 flow_kind=single_candidate + interactive 激活）——
  let stage3Controller = null;
  let stage3CheckpointPath = null;
  const ensureStage3Controller = () => {
    if (!stage3TypedFlowActive(result.flow_kind, runPolicy)) return null;
    if (stage3Controller) return stage3Controller;
    stage3CheckpointPath = path.join(outDir, 'stage3_command_checkpoint.json');
    let checkpoint = null;
    if (fs.existsSync(stage3CheckpointPath)) {
      try {
        checkpoint = JSON.parse(fs.readFileSync(stage3CheckpointPath, 'utf8'));
      } catch {
        checkpoint = null; // checkpoint 损坏时回退确定性重算，不阻断恢复
      }
    }
    stage3Controller = createStage3GateController({
      actions: humanScript,
      campaignRunId: `workitem:${provider}:rep${rep}:${result.issue_id ?? result.workspace_session_id ?? 'unknown'}`,
      checkpoint,
      // command id 一经 materialize 立即落盘，进程重启复用原值。
      persistCheckpoint: (next) => fs.writeFileSync(stage3CheckpointPath, json(next), 'utf8'),
    });
    return stage3Controller;
  };
  const syncStage3Fields = () => {
    if (!stage3Controller) return;
    Object.assign(result, stage3Controller.resultFields());
  };

  const finish = (exitCode = 0) => {
    if (ended) return;
    ended = true;
    if (hardTimer) clearTimeout(hardTimer);
    if (!result.completed && !result.failureClass) result.failureClass = result.error ? 'driver_error' : 'incomplete';
    syncStage3Fields();
    result.usage_by_role = Object.keys(usageByRole).length
      ? usageByRole
      : { usage_unavailable: true };
    applyResultTiming(result, elapsedMs());
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
    if (
      result.flow_kind === 'single_candidate'
      && !singleCandidateOutboundAllowed(message, runPolicy)
    ) {
      fail(
        'single_candidate_outbound_not_allowed',
        `SingleCandidate 仅允许 start_generation 或 interactive human_confirm，拒绝发送: ${String(message?.type)}`,
      );
      return;
    }
    writeLog({ direction: 'out', message: stage3OutboundLogEntry(message) });
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
  const stage3AuditForOutbound = (message) => {
    if (message?.type === 'human_gate_feedback') {
      return {
        type: message.type,
        decision: null,
        command_id: message.command_id,
        feedback_digest: stage3FeedbackDigest(message.feedback),
        feedback_length: typeof message.feedback === 'string' ? message.feedback.length : 0,
      };
    }
    return {
      type: message?.type ?? null,
      decision: message?.decision ?? null,
      command_id: message?.command_id ?? null,
      feedback_digest: null,
      feedback_length: null,
    };
  };
  const deliverStage3Submission = (submission) => {
    if (!submission?.message || ended) return;
    const { message, source } = submission;
    const gateSequence = result.humanGateActions.length + 1;
    const gateAction = humanGateActionAudit({
      sequence: gateSequence,
      enteredAt: humanGateEnteredAt ?? now(),
      elapsedSec: elapsedSec(),
      stage: currentStage,
      reviewVerdict,
      activeNodeId,
      response: {
        decision: message.decision ?? message.type,
        description: null,
        source,
      },
      message,
      sentAt: now(),
    });
    // 脱敏：审计面只留 digest/长度/command_id，不写反馈全文。
    gateAction.message = stage3AuditForOutbound(message);
    result.humanGateActions.push(gateAction);
    writeLog({ event: 'human_gate_action', ...gateAction });
    if (message.type === 'human_gate_feedback') manualRepairRequestsSent += 1;
    send(message);
    syncStage3Fields();
  };
  const handleHumanConfirmStage3 = async () => {
    if (activeNodeType === null) return;
    if (activeNodeType !== 'human_confirm') {
      fail('unexpected_human_confirm', 'interactive human_confirm 未关联到 human_confirm 节点');
      return;
    }
    const controller = ensureStage3Controller();
    if (!controller) return;
    try {
      if (humanScriptConfigured) {
        deliverStage3Submission(controller.onGateWaiting());
        return;
      }
      if (humanMode === 'stdin' && process.stdin.isTTY) {
        const raw = await readHumanStdinAction({
          sequence: result.humanGateActions.length + 1,
          stage: currentStage,
          reviewVerdict,
        });
        if (ended || !isHumanGateStage(currentStage)) return;
        controller.enqueueAction({ decision: raw.decision, description: raw.description });
        deliverStage3Submission(controller.onGateWaiting());
        return;
      }
      // 无脚本时沿用阶段 2 启发式，但 request-change 一律切 typed 编码。
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
      controller.enqueueAction(action.kind === 'confirm'
        ? { decision: 'confirm', description: null }
        : { decision: 'request-change', description: feedback });
      deliverStage3Submission(controller.onGateWaiting());
    } catch (error) {
      fail('human_confirm_input_invalid', error);
    }
  };
  const handleHumanConfirm = async () => {
    if (
      runPolicy !== 'interactive'
      || !activeNodeId
      || result.failureClass
    ) return;
    // 阶段 3：SC+interactive 走 command/turn 级状态机，不再受 node 级“只应答一次”限制。
    if (stage3TypedFlowActive(result.flow_kind, runPolicy)) {
      void handleHumanConfirmStage3();
      return;
    }
    if (humanConfirmSentNodeIds.has(activeNodeId)) return;
    if (activeNodeType === null) return;
    if (activeNodeType !== 'human_confirm') {
      fail('unexpected_human_confirm', 'interactive human_confirm 未关联到 human_confirm 节点');
      return;
    }

    const gateNodeId = activeNodeId;
    const gateSequence = result.humanGateActions.length + 1;
    const gateAction = humanGateActionAudit({
      sequence: gateSequence,
      enteredAt: humanGateEnteredAt ?? now(),
      elapsedSec: elapsedSec(),
      stage: currentStage,
      reviewVerdict,
      activeNodeId: gateNodeId,
    });
    result.humanGateActions.push(gateAction);
    writeLog({ event: 'human_gate_entered', ...gateAction });
    humanConfirmSentNodeIds.add(gateNodeId);
    let response;
    try {
      if (humanScriptConfigured) {
        response = humanConfirmScriptAction(humanScript, gateSequence - 1, runPolicy);
      } else if (humanMode === 'stdin' && process.stdin.isTTY) {
        response = await readHumanStdinAction({
          sequence: gateSequence,
          stage: currentStage,
          reviewVerdict,
        });
      } else {
        const action = humanConfirmAction({
          activeNodeType,
          reviewVerdict,
          reviewComplete: latestReviewComplete,
          humanGateSnapshot: result.human_gate_snapshot,
          manualRepairsUsed: durableRunHistory?.manual_repairs_used ?? 0,
          manualRepairRequestsSent,
        });
        if (action.kind === 'wait') {
          humanConfirmSentNodeIds.delete(gateNodeId);
          result.humanGateActions.pop();
          return;
        }
        if (action.kind === 'fail') {
          fail(action.failureClass, 'interactive human_confirm 的 manual 返修预算已耗尽或节点类型未知');
          return;
        }
        response = action.kind === 'confirm'
          ? { decision: 'confirm', description: null, source: 'legacy_interactive_policy' }
          : { decision: 'request-change', description: feedback, source: 'legacy_interactive_policy' };
      }
    } catch (error) {
      fail('human_confirm_input_invalid', error);
      return;
    }

    // stdin 等待期间可能已切到另一个 gate；不得向旧节点补发决定。
    if (ended || !isHumanGateStage(currentStage) || activeNodeId !== gateNodeId) return;
    const message = humanConfirmMessage(response);
    if (message.decision === 'request-change') manualRepairRequestsSent += 1;
    Object.assign(gateAction, humanGateActionAudit({
      sequence: gateSequence,
      enteredAt: gateAction.enteredAt,
      elapsedSec: gateAction.elapsedSec,
      stage: currentStage,
      reviewVerdict,
      activeNodeId: gateNodeId,
      response,
      message,
      sentAt: now(),
    }));
    writeLog({ event: 'human_gate_action', ...gateAction });
    send(message);
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
    result.provider_start_ledger = protocol.provider_start_ledger;
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
  let confirmedHandoff = null;
  let advanceFinishPending = false;
  const writeHandoffAndFinish = () => {
    if (!confirmedHandoff || ended) return;
    fs.writeFileSync(path.join(outDir, 'handoff.json'), json(confirmedHandoff), 'utf8');
    finish(0);
  };
  const completeConfirmedSession = () => {
    if (completionVerificationStarted) return;
    completionVerificationStarted = true;
    result.completed = true;
    verifyConfirmedPlan(result, elapsedMs)
      .then(({ plan, workItemIds }) => {
        result.confirmed_count = confirmedCountForPlanStatus(plan.status);
        result.work_item_ids = workItemIds;
        result.validator_findings.push(...(Array.isArray(plan.validator_findings) ? plan.validator_findings : []));
        confirmedHandoff = {
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
        // Confirmed 回读后若脚本下一项为 advance：发送 typed command 并等待
        // advance_completed/advance_rejected；否则保持阶段 2 handoff finish 行为。
        const controller = stage3TypedFlowActive(result.flow_kind, runPolicy)
          ? ensureStage3Controller()
          : null;
        if (controller?.currentAction()?.decision === 'advance') {
          const advanceDecision = controller.onConfirmedDurable({
            providerStartLedger: result.provider_start_ledger ?? [],
          });
          syncStage3Fields();
          if (advanceDecision.decision === 'send' && advanceDecision.outbound) {
            advanceFinishPending = true;
            writeLog({
              event: 'stage3_advance_simulation',
              decision: 'send',
              command_id: advanceDecision.commandId,
            });
            send(advanceDecision.outbound);
            return;
          }
        }
        writeHandoffAndFinish();
      })
      .catch((error) => fail('handoff_verification_failed', error));
  };
  const finalizeFromDurableSession = (trigger, closeEvent = null) => {
    if (ended || completionVerificationStarted) {
      // Confirmed 后等待 advance 结果期间传输层断开：plan 已 Confirmed，durable advance
      // 记录的对账留待 8.4b 重连矩阵；此处不悬挂，按已知事实收尾。
      if (advanceFinishPending && !ended) {
        advanceFinishPending = false;
        writeLog({ event: 'stage3_advance_outcome_unknown', trigger });
        writeHandoffAndFinish();
      }
      return true;
    }
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
    const action = terminalSessionAction(
      durable.message,
      awaitingReviewNeedsHumanTerminal,
      true,
      durable.protocol.flow_kind,
    );
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
  const handleStage = (stage, source, stageActiveNodeId = null) => {
    const previousStage = currentStage;
    currentStage = stage;
    // 后端可能先广播 stage_change、后创建确认节点；仅在消息没有携带新 active node 时
    // 清除旧节点。携带 active node 的人工门必须立刻走统一 human_confirm 应答。
    if (shouldClearPendingGateNode(stage, source, stageActiveNodeId, activeNodeType)) {
      activeNodeId = null;
      activeNodeType = null;
    }
    if (isHumanGateStage(stage) && !isHumanGateStage(previousStage)) humanGateEnteredAt = now();
    if (!isHumanGateStage(stage)) humanGateEnteredAt = null;
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
      return;
    }
    // SingleCandidate 的控制面只允许 start_generation；interactive 人工门是唯一例外，
    // 无论服务端使用 human_confirm 或 waiting_for_human，都必须走统一 human_confirm 应答。
    if (isHumanGateStage(stage) && runPolicy === 'interactive') {
      void handleHumanConfirm();
      return;
    }
    if (result.flow_kind === 'single_candidate') return;
    if (stage === 'author_confirm') {
      handleAuthorConfirm();
    }
  };

  hardTimer = setTimeout(() => {
    fail(result.failureClass ?? 'hard_timeout', result.error ?? `硬超时 ${HARD_LIMIT_MS}ms`);
  }, HARD_LIMIT_MS);

  try {
    // 保持既有读取时机：仅由初始 runPolicy=interactive 触发 ARIA_FEEDBACK_FILE。
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

    if (runPolicy === 'interactive') {
      result.humanGateActions ??= [];
      humanScriptConfigured = process.env.ARIA_HUMAN_SCRIPT !== undefined;
      humanScript = parseHumanScript(process.env.ARIA_HUMAN_SCRIPT);
      humanMode = process.env.ARIA_HUMAN_MODE === 'stdin' ? 'stdin' : null;
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
          const action = terminalSessionAction(
            message,
            awaitingReviewNeedsHumanTerminal,
            false,
            protocol.flow_kind,
          );
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
            finalizeFromDurableSession('session_state_terminal');
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
        const legacyDecision = result.flow_kind === 'single_candidate'
          ? legacySingleCandidateDecisionMessage(message, runPolicy)
          : null;
        if (legacyDecision) {
          result.legacy_decision_messages.push(legacyDecision);
          fail(
            'legacy_single_candidate_decision',
            `SingleCandidate 收到旧决策消息，拒绝自动应答: ${String(legacyDecision.type)}`,
          );
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
          const previousActiveNodeId = activeNodeId;
          const previousActiveNodeType = activeNodeType;
          activeNodeId = message.active_node_id;
          activeNodeType = activeNodeTypeForActiveNode(
            nodesById,
            activeNodeId,
            previousActiveNodeId,
            previousActiveNodeType,
          );
          if (isHumanGateStage(currentStage) && activeNodeId !== previousActiveNodeId) {
            humanGateEnteredAt = now();
          }
        }
        switch (message.type) {
        case 'session_state':
        case 'stage_change':
          handleStage(message.stage, message.type, message.active_node_id ?? null);
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
            if (isHumanGateStage(currentStage) && activeNodeType === 'human_confirm') {
              void handleHumanConfirm();
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
            // stage_change 可能早于节点创建；节点补齐后重试统一人工门，仍由 node id 去重。
            if (isHumanGateStage(currentStage)) void handleHumanConfirm();
          }
          break;
        case 'provider_status':
        case 'execution_event':
          break;
        case 'human_gate_turn_open':
        case 'human_gate_turn_completed':
        case 'human_gate_turn_failed':
        case 'human_gate_busy':
        case 'human_gate_closed':
        case 'advance_completed':
        case 'advance_rejected': {
          // 阶段 3 typed 事件只允许出现在 SC+interactive；其他 flow 视为协议回归失败关闭。
          const controller = stage3TypedFlowActive(result.flow_kind, runPolicy)
            ? ensureStage3Controller()
            : null;
          if (!controller) {
            fail(
              'stage3_event_outside_typed_flow',
              `非 typed flow（flow_kind=${String(result.flow_kind)}, run_policy=${String(runPolicy)}）收到 ${message.type}`,
            );
            break;
          }
          const outcome = controller.onInbound(message);
          for (const consumed of outcome.consumed) {
            writeLog({
              event: 'stage3_action_consumed',
              actionIndex: consumed.actionIndex,
              kind: consumed.kind,
              via: consumed.via,
              command_id: message.command_id ?? null,
              turn_id: message.turn_id ?? null,
            });
          }
          if (outcome.outbound) deliverStage3Submission(outcome.outbound);
          // advance 等待期收尾：rejected 不消费脚本动作，但必须无条件收尾（不悬挂至 hard
          // timeout）；completed 仍要求动作确认消费后才收尾。
          const advanceFinishPlan = stage3AdvanceFinishPlan({
            finishPending: advanceFinishPending,
            message,
            consumedKinds: outcome.consumed.map((entry) => entry.kind),
          });
          if (advanceFinishPlan) {
            advanceFinishPending = false;
            controller.noteProviderLedgerAfter(result.provider_start_ledger ?? []);
            syncStage3Fields();
            writeLog(advanceFinishPlan);
            writeHandoffAndFinish();
          }
          syncStage3Fields();
          break;
        }
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
      human_script_grammar: STAGE3_HUMAN_SCRIPT_GRAMMAR,
      advance_simulation: {
        enabled: true,
        scope: 'flow_kind=single_candidate && run_policy=interactive',
        precondition: 'durable Confirmed 回读成功且此前未 advance',
        command: 'advance { command_id }',
        starts_coding_provider: false,
      },
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
  activeNodeTypeForActiveNode,
  advanceSimulationAction,
  applyResultTiming,
  authorConfirmAction,
  campaignCommandId,
  collectUsageByRole,
  confirmedCountForPlanStatus,
  createStage3GateController,
  generationModeSelectionForNode,
  legacySingleCandidateDecisionMessage,
  hasMustFixFindings,
  humanConfirmAction,
  humanConfirmMessage,
  humanConfirmRequestChangeMessage,
  humanConfirmScriptAction,
  humanGateActionAudit,
  isHumanGateStage,
  parseHumanScript,
  shouldClearPendingGateNode,
  shouldConsumeHumanAction,
  singleCandidateOutboundAllowed,
  stage3AdvanceFinishPlan,
  stage3FeedbackDigest,
  stage3HumanMessage,
  stage3OutboundLogEntry,
  stage3TypedFlowActive,
  providerRolesForSelection,
  prepareOptionsForProvider,
  resultTemplate,
  reviewCompleteAction,
  reviewCycleId,
  reviewDecisionAction,
  reviewRepairAction,
  sessionStateProtocol,
  skipOptionalFindingsOption,
  terminalSessionAction,
};
