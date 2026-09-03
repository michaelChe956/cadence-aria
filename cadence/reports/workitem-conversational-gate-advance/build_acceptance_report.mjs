// 阶段 3 Task 8.5 —— 验收报告构建器（acceptance report builder）。
//
// 聚合规则（fail-closed，与 acceptance_report.test.mjs 的负向用例一一对应）：
//   * scenario-evidence.jsonl 必须与 openspec spec 的 14 REQ / 37 scenario
//     原名集合完全相等：缺行 / 多行 / 重复 scenario 均构建失败；
//   * 每行 evidence_refs / test_or_campaign / durable_assertions 非空，且
//     rust:/node:/artifact: 引用必须在仓库内真实可解析（rg / fs）；
//     ledger: 引用要求非空字符串（人工可反查的台账锚点，不机器解析）；
//   * commands.manifest.json 中测试类 command 的 test_count 必须为 >=1 的
//     数字（test_count=0 fail-closed），全部 command 必须有 exit_code 与 log_ref；
//   * real_provider_runs：status=executed* 的跑必须 authorized=true 且带
//     authorization_ref（真实 provider 未声明授权即 fail-closed）；
//   * defer.manifest.json 的每条 residual risk / deferred 必须有 owner 或
//     裁决引用（ruling_ref）；
//   * durable_invariants 只在锚定测试既存在、又被 scenario 证据引用时为 true；
//   * git diff --cached --quiet 非 0（存在 staged files）即 fail-closed。
//
// 生成物 acceptance-report.json 中的 commit / worktree_status_* 来自
// commands.manifest.json 的 meta（记录为生成时刻的被验收实现 HEAD 与前后
// 仓库状态），构建器不读取易漂移的实时 HEAD，保证已提交 JSON 可复算一致。
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
export const repoRoot = path.resolve(here, '..', '..', '..');

// spec 原名集合：openspec/changes/workitem-conversational-gate-advance/specs/*/spec.md
export const SPEC_SCENARIOS = [
  { req_id: 'REQ-CG-01', scenario: '反馈回合触发一次修订' },
  { req_id: 'REQ-CG-01', scenario: '客户端未收到响应重发同一命令' },
  { req_id: 'REQ-CG-02', scenario: '门内并发反馈被拒绝' },
  { req_id: 'REQ-CG-02', scenario: '回合进行中收到终止决定被拒绝' },
  { req_id: 'REQ-CG-02', scenario: 'provider 瞬断恢复' },
  { req_id: 'REQ-CG-03', scenario: '修订成功回呈' },
  { req_id: 'REQ-CG-03', scenario: '修订不过校验' },
  { req_id: 'REQ-CG-03', scenario: '反馈超长零副作用拒绝' },
  { req_id: 'REQ-CG-03', scenario: '同指纹重现回同一门' },
  { req_id: 'REQ-CG-03', scenario: '不触碰 legacy 约束链' },
  { req_id: 'REQ-CG-04', scenario: 'approve 不绕过 compile' },
  { req_id: 'REQ-CG-04', scenario: '预算耗尽拒绝新反馈' },
  { req_id: 'REQ-CG-05', scenario: 'turn 预留原子性' },
  { req_id: 'REQ-CG-05', scenario: '回合中断线重连' },
  { req_id: 'REQ-CG-06', scenario: '接管 auto stopped session' },
  { req_id: 'REQ-ADV-01', scenario: 'Confirmed plan 推进到 coding 就绪' },
  { req_id: 'REQ-ADV-01', scenario: '前置不满足零副作用拒绝' },
  { req_id: 'REQ-ADV-02', scenario: '同一命令键重发' },
  { req_id: 'REQ-ADV-02', scenario: '不同命令键重复推进同一 plan' },
  { req_id: 'REQ-ADV-02', scenario: '已有失败/中止 attempt 时不隐式重建' },
  { req_id: 'REQ-ADV-02', scenario: '初始化中断恢复' },
  { req_id: 'REQ-ADV-03', scenario: 'SC plan 绕过 advance 直接进入 coding 被拒' },
  { req_id: 'REQ-ADV-03', scenario: 'legacy 入口行为零变化' },
  { req_id: 'REQ-GCE-01', scenario: '依赖未就绪不跳过' },
  { req_id: 'REQ-GCE-01', scenario: '全部 pending 未就绪进入等待态' },
  { req_id: 'REQ-GCE-01', scenario: 'handoff 与 plan binding 不匹配 fail-closed' },
  { req_id: 'REQ-GCE-01', scenario: '依赖图异常 fail-closed' },
  { req_id: 'REQ-GCE-01', scenario: 'legacy attempt 不适用依赖门' },
  { req_id: 'REQ-GCE-02', scenario: 'unit 失败重试不新建 attempt' },
  { req_id: 'REQ-GCE-03', scenario: 'plan 缺陷修订后回到 coding' },
  { req_id: 'REQ-GCE-03', scenario: 'amendment 回合挂在原 plan session' },
  { req_id: 'REQ-GCE-03', scenario: 'amendment 批准后恢复原 attempt' },
  { req_id: 'REQ-GCE-04', scenario: '每个 Work Item 的进展与成果可见' },
  { req_id: 'REQ-WSC-01', scenario: '自动化 campaign 无人工干预到达终态' },
  { req_id: 'REQ-WSC-01', scenario: '生成模式不再询问' },
  { req_id: 'REQ-WSC-01', scenario: 'interactive 门内多轮修订后批准' },
  { req_id: 'REQ-WSC-01', scenario: 'Confirmed 后经 advance 进入 coding 就绪' },
];

export const EXPECTED_SCENARIO_COUNTS = SPEC_SCENARIOS.reduce((acc, s) => {
  acc[s.req_id] = (acc[s.req_id] ?? 0) + 1;
  return acc;
}, {});

// durable invariant → 锚定测试（必须存在且被 scenario 证据引用，才允许置 true）。
const INVARIANT_TESTS = {
  budget_exactly_once: [
    'human_gate_reservation_cas_writes_turn_budget_and_provider_key_atomically',
    'campaign_stage3_turn_reservation_crash_recovers_exactly_once',
  ],
  provider_ledger_reconciled: [
    'conversational_gate_revision_ledger_counts_real_starts_only',
    'campaign_stage3_advance_confirmed_plan_is_ready_without_provider_start',
  ],
  event_prefix_immutable: [
    'conversational_gate_recovery_preserves_event_prefix_and_budget',
    'conversational_gate_takeover_replay_returns_same_child_without_parent_mutation',
  ],
  advance_unique_attempt: [
    'persist_advance_record_if_absent_replays_by_plan_before_new_command',
    'campaign_stage3_advance_journal_crash_resumes_same_identity',
  ],
  single_active_unit: [
    'sc_group_dependency_gate_allows_only_one_active_unit_under_concurrency',
    'campaign_stage3_inflight_rejects_feedback_approve_and_abandon_as_busy',
  ],
  amendment_same_attempt: [
    'group_amendment_plan_defect_stays_on_same_attempt',
    'campaign_stage3_amendment_returns_to_same_attempt_via_original_plan_session',
  ],
};

function rg(needle, args) {
  try {
    // rg 15.x：--fixed-strings 是修饰 flag，pattern 必须位于 path 之前。
    execFileSync('rg', ['--fixed-strings', needle, ...args], { cwd: repoRoot, stdio: ['ignore', 'ignore', 'ignore'] });
    return true;
  } catch {
    return false;
  }
}

export function defaultResolvers() {
  return {
    rust: (name) => rg(`fn ${name}(`, ['src/', '-g', '*.rs']),
    node: (name) => rg(`test('${name}'`, ['cadence/reports/workitem-coding-campaign/', '-g', '*.mjs']),
    artifact: (rel) => fs.existsSync(path.join(here, rel)),
  };
}

function runGit(args) {
  try {
    execFileSync('git', args, { cwd: repoRoot, stdio: ['ignore', 'ignore', 'ignore'] });
    return 0;
  } catch (error) {
    return error.status ?? 1;
  }
}

export function checkNoStagedFiles(run = runGit) {
  return run(['diff', '--cached', '--quiet']) === 0;
}

function resolveRef(ref, resolvers, errors, context) {
  const kind = ref.slice(0, ref.indexOf(':'));
  const value = ref.slice(kind.length + 1);
  if (!value) {
    errors.push(`evidence ref without value (${context}): ${ref}`);
    return;
  }
  if (kind === 'ledger') return; // 台账锚点：非空即可，人工反查 progress.md/任务报告
  const resolver = { rust: resolvers.rust, node: resolvers.node, artifact: resolvers.artifact }[kind];
  if (!resolver) {
    errors.push(`unknown evidence ref kind (${context}): ${ref}`);
    return;
  }
  if (!resolver(value)) errors.push(`unresolvable ${kind} evidence (${context}): ${value}`);
}

export function buildAcceptanceReport({
  scenarioEvidenceText,
  commandsManifest,
  deferManifest,
  resolvers = defaultResolvers(),
}) {
  const errors = [];

  // ---- scenario-evidence.jsonl ----
  const lines = scenarioEvidenceText.trim().length === 0 ? [] : scenarioEvidenceText.trim().split('\n');
  const records = [];
  const seen = new Map();
  lines.forEach((line, index) => {
    let record;
    try {
      record = JSON.parse(line);
    } catch (error) {
      errors.push(`scenario-evidence.jsonl line ${index + 1} is not valid JSON: ${error.message}`);
      return;
    }
    records.push(record);
    const key = `${record.req_id}::${record.scenario}`;
    if (seen.has(key)) errors.push(`duplicate scenario line: ${key} (lines ${seen.get(key) + 1} and ${index + 1})`);
    if (!seen.has(key)) seen.set(key, index + 1);
  });
  const present = new Set(records.map((r) => `${r.req_id}::${r.scenario}`));
  for (const expected of SPEC_SCENARIOS) {
    const key = `${expected.req_id}::${expected.scenario}`;
    if (!present.has(key)) errors.push(`missing scenario for ${key} (${present.size}/37 present)`);
  }
  for (const key of present) {
    if (!SPEC_SCENARIOS.some((s) => `${s.req_id}::${s.scenario}` === key)) {
      errors.push(`unexpected scenario not in spec: ${key}`);
    }
  }
  const allTestRefs = new Set();
  for (const record of records) {
    const context = `${record.req_id}::${record.scenario}`;
    for (const field of ['req_id', 'scenario', 'status', 'provider_authorization']) {
      if (typeof record[field] !== 'string' || record[field].length === 0) {
        errors.push(`scenario ${context}: field ${field} must be a non-empty string`);
      }
    }
    if (!['passed', 'failed', 'not_run'].includes(record.status)) {
      errors.push(`scenario ${context}: invalid status ${record.status}`);
    }
    if (!['fake_deterministic_only', 'authorized_real_attempted_no_confirmed_run'].includes(record.provider_authorization)) {
      errors.push(`scenario ${context}: invalid provider_authorization ${record.provider_authorization}`);
    }
    for (const field of ['test_or_campaign', 'evidence_refs', 'durable_assertions']) {
      if (!Array.isArray(record[field]) || record[field].length === 0) {
        errors.push(`scenario ${context}: no ${field} evidence`);
      }
    }
    for (const ref of [...(record.test_or_campaign ?? []), ...(record.evidence_refs ?? [])]) {
      resolveRef(ref, resolvers, errors, context);
      if (String(ref).startsWith('rust:') || String(ref).startsWith('node:')) allTestRefs.add(ref);
    }
  }

  // ---- commands.manifest.json ----
  const { commands = [], real_provider_runs = [], meta = {} } = commandsManifest ?? {};
  const commandEntries = [];
  for (const command of commands) {
    if (typeof command.command !== 'string' || command.command.length === 0) errors.push('command entry without command text');
    if (typeof command.exit_code !== 'number') errors.push(`command missing exit_code: ${command.command}`);
    if (command.is_test_command !== false && !(Number.isInteger(command.test_count) && command.test_count >= 1)) {
      errors.push(`test command must have test_count>=1 (test_count=0 fail-closed): ${command.command}`);
    }
    if (typeof command.log_ref !== 'string' || command.log_ref.length === 0) {
      errors.push(`command missing log_ref: ${command.command}`);
    } else {
      resolveRef(command.log_ref, resolvers, errors, `command ${command.command}`);
    }
    commandEntries.push({
      command: command.command,
      exit_code: command.exit_code,
      test_count: command.is_test_command === false ? 0 : command.test_count,
      is_test_command: command.is_test_command !== false,
      note: command.note ?? '',
      log_ref: command.log_ref,
    });
  }
  const providerRuns = [];
  for (const run of real_provider_runs) {
    if (typeof run.provider !== 'string' || run.provider.length === 0) errors.push('real provider run without provider');
    if (typeof run.authorized !== 'boolean') errors.push(`real provider run missing authorized flag: ${run.provider} ${run.run ?? ''}`);
    if (String(run.status).startsWith('executed')) {
      if (run.authorized !== true) errors.push(`executed real provider run without declared authorization: ${run.provider} ${run.run ?? ''}`);
      if (!run.authorization_ref) errors.push(`executed real provider run missing authorization_ref: ${run.provider} ${run.run ?? ''}`);
      if (run.status === 'executed_no_confirmed_run' && !run.failure_class) {
        errors.push(`executed_no_confirmed_run missing failure_class: ${run.provider} ${run.run ?? ''}`);
      }
    }
    providerRuns.push({ ...run });
  }

  // ---- defer.manifest.json ----
  const risks = [];
  const deferred = [];
  for (const [field, sink] of [['residual_risks', risks], ['deferred', deferred]]) {
    for (const item of deferManifest?.[field] ?? []) {
      if (!item.title) errors.push(`${field} item missing title`);
      if (!item.owner && !item.ruling_ref) errors.push(`${field} item missing owner or ruling_ref: ${item.title}`);
      sink.push({ ...item });
    }
  }

  // ---- durable invariants ----
  const durableInvariants = {};
  for (const [invariant, anchors] of Object.entries(INVARIANT_TESTS)) {
    durableInvariants[invariant] = anchors.every((name) => {
      const exists = resolvers.rust(name);
      if (!exists) errors.push(`durable invariant ${invariant}: unresolvable anchor test ${name}`);
      const cited = [...allTestRefs].some((ref) => ref === `rust:${name}`);
      if (!cited) errors.push(`durable invariant ${invariant}: anchor test ${name} not cited by any scenario evidence`);
      return exists && cited;
    });
  }

  // ---- 仓库纪律：staged files fail-closed ----
  const noStagedFiles = checkNoStagedFiles();
  if (!noStagedFiles) errors.push('staged files present (git diff --cached --quiet failed)');

  // ---- 聚合计数（不得手填：由逐行 status 汇总）----
  const scenarioTally = { passed: 0, failed: 0, not_run: 0 };
  for (const record of records) scenarioTally[record.status] += 1;
  const requirementTally = { passed: 0, failed: 0, not_run: 0 };
  for (const reqId of Object.keys(EXPECTED_SCENARIO_COUNTS)) {
    const rows = records.filter((r) => r.req_id === reqId);
    const status = rows.some((r) => r.status === 'failed') ? 'failed'
      : rows.some((r) => r.status === 'not_run') ? 'not_run'
        : rows.length === EXPECTED_SCENARIO_COUNTS[reqId] ? 'passed' : 'not_run';
    requirementTally[status] += 1;
  }

  const report = {
    change: 'workitem-conversational-gate-advance',
    commit: meta.commit ?? '',
    worktree_status_before: meta.worktree_status_before ?? [],
    worktree_status_after: meta.worktree_status_after ?? [],
    requirements: { total: 14, ...requirementTally },
    scenarios: { total: 37, ...scenarioTally },
    commands: commandEntries,
    real_provider_runs: providerRuns,
    durable_invariants: durableInvariants,
    changed_files: meta.changed_files ?? [],
    residual_risks: risks,
    deferred,
    review_findings: meta.review_findings ?? [],
    no_staged_files: noStagedFiles,
  };
  if (errors.length > 0) return { ok: false, errors, report };
  return { ok: true, errors: [], report };
}

// CLI：node build_acceptance_report.mjs [--check]
// 默认从本目录读取 manifests 并写出 acceptance-report.json；--check 只比对不写盘。
export function loadInputs() {
  return {
    scenarioEvidenceText: fs.readFileSync(path.join(here, 'scenario-evidence.jsonl'), 'utf8'),
    commandsManifest: JSON.parse(fs.readFileSync(path.join(here, 'commands.manifest.json'), 'utf8')),
    deferManifest: JSON.parse(fs.readFileSync(path.join(here, 'defer.manifest.json'), 'utf8')),
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const built = buildAcceptanceReport(loadInputs());
  const serialized = `${JSON.stringify(built.report, null, 2)}\n`;
  if (process.argv.includes('--check')) {
    const committed = fs.readFileSync(path.join(here, 'acceptance-report.json'), 'utf8');
    if (!built.ok || committed !== serialized) {
      console.error(`check failed:\n${built.errors.join('\n')}${built.ok ? '' : ''}${committed !== serialized ? '\nacceptance-report.json drifted from builder output' : ''}`);
      process.exit(1);
    }
    console.log('acceptance-report.json matches builder output (14 REQ / 37 scenarios, fail-closed checks clean)');
    process.exit(0);
  }
  if (!built.ok) {
    console.error(`build failed:\n${built.errors.join('\n')}`);
    process.exit(1);
  }
  fs.writeFileSync(path.join(here, 'acceptance-report.json'), serialized);
  console.log(`wrote acceptance-report.json (${built.report.requirements.passed}/14 REQ passed, ${built.report.scenarios.passed}/37 scenarios passed)`);
}
