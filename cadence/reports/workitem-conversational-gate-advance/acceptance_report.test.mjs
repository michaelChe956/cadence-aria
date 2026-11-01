// 阶段 3 Task 8.5 —— 验收报告构建器（acceptance report builder）的 fail-closed 测试。
//
// 纯 Node：不启动服务、不发网络请求、不要求 credential。
// 校验对象：本目录下由 build_acceptance_report.mjs 聚合生成的
// scenario-evidence.jsonl / acceptance-report.json / commands.manifest.json /
// defer.manifest.json，以及 README.md / defer-ledger.md 的链接可解析性。
//
// fail-closed 形态（brief Step 1 全量）：
//   1) 缺行（37 场景 / 14 REQ 逐一对照 spec 原名集合）；
//   2) 重复 scenario；
//   3) 无 evidence（evidence_refs / test_or_campaign / durable_assertions 为空，
//      或引用的 rust:/node:/artifact: 证据在仓库中不可解析）；
//   4) test_count=0（测试类 command 的 test_count 必须非零）；
//   5) 真实 provider 未声明授权（status=executed* 但 authorized≠true 或缺授权引用）；
//   6) staged files（git diff --cached --quiet 非 0）。
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

import {
  EXPECTED_SCENARIO_COUNTS,
  buildAcceptanceReport,
  checkNoStagedFiles,
} from './build_acceptance_report.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '..', '..', '..');
const read = (p) => fs.readFileSync(path.join(here, p), 'utf8');

function realInputs() {
  return {
    scenarioEvidenceText: read('scenario-evidence.jsonl'),
    commandsManifest: JSON.parse(read('commands.manifest.json')),
    deferManifest: JSON.parse(read('defer.manifest.json')),
  };
}

function mutate(lines, transform) {
  const next = transform(lines.slice());
  return next.map((o) => JSON.stringify(o)).join('\n') + '\n';
}

test('campaign_stage3_acceptance_report_requires_14_requirements_and_37_scenarios', () => {
  const inputs = realInputs();
  const built = buildAcceptanceReport(inputs);
  assert.equal(built.ok, true, `builder errors: ${built.errors.join('; ')}`);
  const { report } = built;

  // 缺行：14 REQ / 37 scenario 与 spec 原名逐一对应。
  assert.equal(report.requirements.total, 14);
  assert.equal(report.scenarios.total, 37);
  assert.equal(report.requirements.passed + report.requirements.failed + report.requirements.not_run, 14);
  assert.equal(report.scenarios.passed + report.scenarios.failed + report.scenarios.not_run, 37);
  assert.deepEqual(EXPECTED_SCENARIO_COUNTS, {
    'REQ-CG-01': 2, 'REQ-CG-02': 3, 'REQ-CG-03': 5, 'REQ-CG-04': 2, 'REQ-CG-05': 2, 'REQ-CG-06': 1,
    'REQ-ADV-01': 2, 'REQ-ADV-02': 4, 'REQ-ADV-03': 2,
    'REQ-GCE-01': 5, 'REQ-GCE-02': 1, 'REQ-GCE-03': 3, 'REQ-GCE-04': 1,
    'REQ-WSC-01': 4,
  });

  // 每行必备字段齐备且 evidence 可解析（无 evidence fail-closed 已在 builder 内）。
  const lines = inputs.scenarioEvidenceText.trim().split('\n').map((l) => JSON.parse(l));
  assert.equal(lines.length, 37);
  const seen = new Set();
  for (const line of lines) {
    for (const field of ['req_id', 'scenario', 'test_or_campaign', 'status', 'evidence_refs', 'durable_assertions', 'provider_authorization']) {
      assert.ok(line[field] !== undefined && line[field] !== null, `line missing ${field}: ${line.scenario}`);
    }
    assert.ok(line.evidence_refs.length >= 1, `no evidence refs: ${line.scenario}`);
    assert.ok(line.durable_assertions.length >= 1, `no durable assertions: ${line.scenario}`);
    assert.ok(line.test_or_campaign.length >= 1, `no test_or_campaign: ${line.scenario}`);
    const key = `${line.req_id}::${line.scenario}`;
    assert.ok(!seen.has(key), `duplicate scenario line: ${key}`);
    seen.add(key);
  }

  // 与已提交的 acceptance-report.json 逐字段一致（漂移即 fail）。
  const committed = JSON.parse(read('acceptance-report.json'));
  assert.deepEqual(report, committed);

  // 机器可读闭环：commands / durable invariants / defer 结构由 builder 保证，
  // 这里抽验关键字段。
  assert.equal(report.no_staged_files, true);
  assert.equal(report.change, 'workitem-conversational-gate-advance');
  // commit 记录被验收实现态 HEAD（验收证据 commit 只含本目录产物，不改代码行为）：
  // 它必须是当前 HEAD 的祖先（证据 commit 落盘前等于 HEAD 本身，落盘后为父代）。
  const acceptedHead = report.commit;
  assert.match(acceptedHead, /^[0-9a-f]{40}$/);
  let isAncestor = true;
  try {
    execFileSync('git', ['merge-base', '--is-ancestor', acceptedHead, 'HEAD'], { cwd: repoRoot, stdio: ['ignore', 'ignore', 'ignore'] });
  } catch {
    isAncestor = false;
  }
  assert.equal(isAncestor, true, `accepted implementation HEAD ${acceptedHead} must be an ancestor of current HEAD`);
  for (const invariant of Object.values(report.durable_invariants)) {
    assert.equal(invariant, true);
  }
  for (const risk of report.residual_risks) {
    assert.ok(risk.owner, `residual risk missing owner: ${JSON.stringify(risk)}`);
  }
  for (const item of report.deferred) {
    assert.ok(item.owner, `deferred item missing owner: ${JSON.stringify(item)}`);
  }
  // 真实 provider 账：每条要么未执行，要么 authorized=true 且带授权引用。
  for (const run of report.real_provider_runs) {
    assert.equal(typeof run.authorized, 'boolean');
    if (String(run.status).startsWith('executed')) {
      assert.equal(run.authorized, true, `executed run must be authorized: ${JSON.stringify(run)}`);
      assert.ok(run.authorization_ref, `executed run missing authorization_ref: ${JSON.stringify(run)}`);
    }
  }
});

test('campaign_stage3_acceptance_report_fails_closed_on_missing_scenario_line', () => {
  const inputs = realInputs();
  const lines = inputs.scenarioEvidenceText.trim().split('\n').map((l) => JSON.parse(l));
  const built = buildAcceptanceReport({
    ...inputs,
    scenarioEvidenceText: mutate(lines, (ls) => ls.filter((l) => l.scenario !== '修订成功回呈')),
  });
  assert.equal(built.ok, false);
  assert.ok(built.errors.some((e) => e.includes('missing scenario') || e.includes('30')), JSON.stringify(built.errors));
});

test('campaign_stage3_acceptance_report_fails_closed_on_duplicate_scenario', () => {
  const inputs = realInputs();
  const lines = inputs.scenarioEvidenceText.trim().split('\n').map((l) => JSON.parse(l));
  const built = buildAcceptanceReport({
    ...inputs,
    scenarioEvidenceText: mutate(lines, (ls) => [...ls, ls.find((l) => l.scenario === '同一命令键重发')]),
  });
  assert.equal(built.ok, false);
  assert.ok(built.errors.some((e) => e.includes('duplicate')), JSON.stringify(built.errors));
});

test('campaign_stage3_acceptance_report_fails_closed_on_missing_evidence', () => {
  const inputs = realInputs();
  const lines = inputs.scenarioEvidenceText.trim().split('\n').map((l) => JSON.parse(l));
  const built = buildAcceptanceReport({
    ...inputs,
    scenarioEvidenceText: mutate(lines, (ls) => ls.map((l) => (
      l.scenario === '预算耗尽拒绝新反馈'
        ? { ...l, evidence_refs: [], durable_assertions: [], test_or_campaign: [] }
        : l
    ))),
  });
  assert.equal(built.ok, false);
  assert.ok(built.errors.some((e) => e.includes('evidence')), JSON.stringify(built.errors));
  // 引用不存在的测试同样 fail-closed。
  const built2 = buildAcceptanceReport({
    ...inputs,
    scenarioEvidenceText: mutate(lines, (ls) => ls.map((l) => (
      l.scenario === '预算耗尽拒绝新反馈'
        ? { ...l, evidence_refs: ['rust:does_not_exist_anywhere_test'] }
        : l
    ))),
  });
  assert.equal(built2.ok, false);
  assert.ok(built2.errors.some((e) => e.includes('unresolvable') || e.includes('does_not_exist_anywhere_test')), JSON.stringify(built2.errors));
});

test('campaign_stage3_acceptance_report_fails_closed_on_zero_test_count', () => {
  const inputs = realInputs();
  const zeroCount = JSON.parse(JSON.stringify(inputs.commandsManifest));
  const mutated = zeroCount.commands.map((c) => (
    c.command.includes('--lib') ? { ...c, test_count: 0 } : c
  ));
  const built = buildAcceptanceReport({
    ...inputs,
    commandsManifest: { ...zeroCount, commands: mutated },
  });
  assert.equal(built.ok, false);
  assert.ok(built.errors.some((e) => e.includes('test_count')), JSON.stringify(built.errors));
});

test('campaign_stage3_acceptance_report_fails_closed_on_unauthorized_real_provider_run', () => {
  const inputs = realInputs();
  const manifest = JSON.parse(JSON.stringify(inputs.commandsManifest));
  manifest.real_provider_runs.push({
    provider: 'codex', run: 'rep9', authorized: false,
    status: 'executed_no_confirmed_run', failure_class: 'x', evidence_ref: 'ledger:none',
  });
  const built = buildAcceptanceReport({ ...inputs, commandsManifest: manifest });
  assert.equal(built.ok, false);
  assert.ok(built.errors.some((e) => e.includes('authorization')), JSON.stringify(built.errors));
});

test('campaign_stage3_acceptance_report_fails_closed_on_staged_files', () => {
  // 注入伪 git 执行器：git diff --cached --quiet 返回 1。
  assert.equal(checkNoStagedFiles((args) => (args.includes('--cached') ? 1 : 0)), false);
  // 主门禁：当前仓库必须无 staged files。
  assert.equal(checkNoStagedFiles(), true);
});

test('campaign_stage3_acceptance_report_markdown_links_resolve', () => {
  for (const doc of ['README.md', 'defer-ledger.md']) {
    const text = read(doc);
    const links = [...text.matchAll(/\]\(([^)#?\s]+)(?:#[^)\s]*)?\)/g)].map((m) => m[1]);
    assert.ok(links.length >= 1, `${doc} has no relative links to check`);
    for (const link of links) {
      if (/^[a-z]+:\/\//.test(link)) continue; // 外部链接不在机器校验范围。
      const target = path.resolve(here, link);
      assert.ok(fs.existsSync(target), `${doc} broken link: ${link}`);
    }
  }
});
