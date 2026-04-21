import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

export type FakeBinaryOptions = {
  codexExitCode?: number;
  reviewVerdict?: 'passed' | 'failed' | 'needs_patch';
  testVerdict?: 'passed' | 'failed' | 'needs_patch';
  claudeMode?: 'pass' | 'invalid' | 'mismatch-task' | 'mismatch-result-set';
  writeArtifactToDisk?: boolean;
};

let fakeBinDir = '';

export async function createFakeBinaries(
  options: FakeBinaryOptions = {},
  originalPath: string = process.env.PATH ?? ''
): Promise<string> {
  const {
    codexExitCode = 0,
    reviewVerdict = 'passed',
    testVerdict = 'passed',
    claudeMode = 'pass',
    writeArtifactToDisk = false,
  } = options;

  fakeBinDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-fake-bins-'));

  const codexPath = path.join(fakeBinDir, 'codex');
  const claudePath = path.join(fakeBinDir, 'claude');

  const codexScript = `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');

const args = process.argv.slice(2);
let outputPath = '';
let promptContent = '';

for (let index = 0; index < args.length; index += 1) {
  const arg = args[index];
  if (arg === 'exec' || arg === '--full-auto') continue;
  if (arg === '-C') {
    index += 1;
    continue;
  }
  if (arg === '--output-last-message') {
    outputPath = args[index + 1] ?? '';
    index += 1;
    continue;
  }
  promptContent = arg;
}

const prompt = promptContent;
const taskId = (prompt.match(/^task_id: (.+)$/m) ?? [])[1] ?? 'unknown';

if (process.env.ARIA_FAKE_CODEX_CWD_LOG) {
  fs.appendFileSync(process.env.ARIA_FAKE_CODEX_CWD_LOG, process.cwd() + '\\n', 'utf8');
}

fs.appendFileSync(path.join(process.cwd(), 'codex-invocation.log'), args.join('\\n') + '\\n', 'utf8');

if (process.env.ARIA_FAKE_CODEX_FAIL === '1') {
  process.stderr.write('fake codex failed\\n');
  process.exit(1);
}

if (process.env.ARIA_FAKE_CODEX_MODE === 'rename') {
  fs.mkdirSync(path.join(process.cwd(), 'src'), { recursive: true });
  fs.renameSync(
    path.join(process.cwd(), 'src', 'rename-old.ts'),
    path.join(process.cwd(), 'src', 'rename-new.ts')
  );
}

fs.writeFileSync(outputPath, [
  'task_id: ' + taskId,
  'exec_unit_id: exec-01',
  'status: succeeded',
  'changed_files:',
  '  - src/index.ts',
  'summary: fake codex exec',
  'capabilities_used:',
  '  - codex',
  'openspec_refs_consumed:',
  '  - artifacts/spec-artifact.md',
  'superpowers_refs_consumed:',
  '  - test-driven-development',
  '  - verification-before-completion',
  'degraded: false',
  'degradation_reason: null',
  'started_at: 2026-04-19T00:00:00.000Z',
  'finished_at: 2026-04-19T00:00:01.000Z',
  ''
].join('\\n'), 'utf8');
process.exit(${codexExitCode});
`;

  const claudeScript = `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');

const mode = process.env.ARIA_FAKE_CLAUDE_MODE ?? '${claudeMode}';
const prompt = process.argv.slice(2).join(' ');
const taskId = (prompt.match(/task_id: (.+?)(?:\\n|$)/m) ?? [])[1] ?? 'unknown';
const resultSetId = (prompt.match(/result_set_id: (.+?)(?:\\n|$)/m) ?? [])[1] ?? 'result-set-unknown';

if (process.env.ARIA_FAKE_CLAUDE_CWD_LOG) {
  fs.appendFileSync(process.env.ARIA_FAKE_CLAUDE_CWD_LOG, process.cwd() + '\\n', 'utf8');
}

fs.appendFileSync(path.join(process.cwd(), 'claude-invocation.log'), prompt + '\\n', 'utf8');

if (process.env.ARIA_FAKE_CLAUDE_DEBUG_LOG) {
  fs.appendFileSync(process.env.ARIA_FAKE_CLAUDE_DEBUG_LOG, 'prompt=' + JSON.stringify(prompt) + '\\n', 'utf8');
}

if (mode === 'invalid') {
  console.error('not-yaml');
  process.exit(0);
}

const reportTaskId = mode === 'mismatch-task' ? 'aria-19990101-001' : taskId;
const reportResultSetId = mode === 'mismatch-result-set' ? 'result-set-mismatch-01' : resultSetId;
const reviewVerdict = '${reviewVerdict}';
const testVerdict = '${testVerdict}';

const isReview = prompt.includes('Claude Code Review Prompt');
const yaml = isReview
  ? [
      'task_id: ' + reportTaskId,
      'result_set_id: ' + reportResultSetId,
      'exec_units_reviewed:',
      '  - exec-01',
      'baseline_refs:',
      '  - artifacts/spec-artifact.md',
      '  - artifacts/plan-brief.md',
      'method_refs:',
      '  - verification-before-completion',
      'blockers: []',
      'suggestions: []',
      'verdict: ' + reviewVerdict,
      'producer: claude-code',
      'source_capabilities:',
      '  - OpenSpec',
      '  - superpowers',
      'generated_at: 2026-04-19T00:00:02.000Z',
      ''
    ].join('\\n')
  : [
      'task_id: ' + reportTaskId,
      'result_set_id: ' + reportResultSetId,
      'exec_units_tested:',
      '  - exec-01',
      'baseline_refs:',
      '  - artifacts/spec-artifact.md',
      '  - artifacts/plan-brief.md',
      'method_refs:',
      '  - test-driven-development',
      '  - verification-before-completion',
      'commands_run:',
      '  - pnpm check',
      '  - pnpm test',
      'failures: []',
      'passed_count: 2',
      'failed_count: 0',
      'verdict: ' + testVerdict,
      'producer: claude-code',
      'source_capabilities:',
      '  - OpenSpec',
      '  - superpowers',
      'generated_at: 2026-04-19T00:00:03.000Z',
      ''
    ].join('\\n');

if (process.env.ARIA_FAKE_CLAUDE_DEBUG_LOG) {
  fs.appendFileSync(process.env.ARIA_FAKE_CLAUDE_DEBUG_LOG, 'yaml=' + JSON.stringify(yaml) + '\\n', 'utf8');
}

const artifactName = isReview ? 'review-report.yaml' : 'test-report.yaml';
const artifactRoot = process.env.ARIA_FAKE_ARTIFACT_ROOT ?? process.cwd();
const artifactPath = path.join(artifactRoot, 'cadence', 'cache', 'aria', 'tasks', taskId, 'artifacts', artifactName);

if (${writeArtifactToDisk} || process.env.ARIA_FAKE_ARTIFACT_ROOT) {
  fs.mkdirSync(path.dirname(artifactPath), { recursive: true });
  fs.writeFileSync(artifactPath, yaml + '\\n', 'utf8');
}

process.stdout.write(yaml + '\\n', () => process.exit(0));
`;

  await fs.writeFile(codexPath, codexScript, 'utf8');
  await fs.writeFile(claudePath, claudeScript, 'utf8');
  await fs.chmod(codexPath, 0o755);
  await fs.chmod(claudePath, 0o755);

  process.env.PATH = `${fakeBinDir}${path.delimiter}${originalPath}`;

  return fakeBinDir;
}

export async function cleanupFakeBinaries(): Promise<void> {
  if (fakeBinDir) {
    await fs.rm(fakeBinDir, { recursive: true, force: true });
    fakeBinDir = '';
  }
}
