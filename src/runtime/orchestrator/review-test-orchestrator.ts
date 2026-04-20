import fs from 'node:fs/promises';
import path from 'node:path';

import { runClaudeCode } from '../../adapters/claude-code/claude-code-adapter.js';
import {
  reviewReportSchema,
  testReportSchema,
  type ReviewReportArtifact,
  type TestReportArtifact
} from '../../schemas/runtime-artifact-schema.js';
import { parseYaml, stringifyYaml } from '../../utils/yaml.js';
import { readState } from '../persistence/state-repository.js';
import { getTaskArtifactsDir } from '../persistence/paths.js';
import {
  validateExecutionContextBundle,
  validateReviewReport,
  validateTestReport
} from '../contracts/contract-validator.js';
import { resolveWorkspacePath } from '../../utils/workspace.js';

function buildReviewPrompt(taskId: string, resultSetId: string): string {
  return [
    '# Claude Code Review Prompt',
    `task_id: ${taskId}`,
    `result_set_id: ${resultSetId}`,
    '请读取当前任务工件，检查 exec 结果、spec 与 plan 的一致性。',
    '请运行必要的检查命令；如无法执行，请在 blockers 中明确说明，并给出 failed verdict。',
    '不要修改任何文件，不要解释流程。',
    '只输出合法 YAML，字段必须完整：task_id、result_set_id、exec_units_reviewed、baseline_refs、method_refs、blockers、suggestions、verdict、producer、source_capabilities、generated_at。'
  ].join('\n');
}

function buildTestPrompt(taskId: string, resultSetId: string): string {
  return [
    '# Claude Code Test Prompt',
    `task_id: ${taskId}`,
    `result_set_id: ${resultSetId}`,
    '请读取当前任务工件并运行必要的验证命令，至少覆盖 contract 中要求的检查。',
    '如无法执行命令或验证失败，请在 failures 中记录，并输出 failed verdict。',
    '不要修改任何文件，不要解释流程。',
    '只输出合法 YAML，字段必须完整：task_id、result_set_id、exec_units_tested、baseline_refs、method_refs、commands_run、failures、passed_count、failed_count、verdict、producer、source_capabilities、generated_at。'
  ].join('\n');
}

function parseReviewReport(input: {
  taskId: string;
  resultSetId: string;
  specRef: string;
  planRef: string;
  output: string;
}): ReviewReportArtifact {
  try {
    return validateReviewReport(reviewReportSchema.parse(parseYaml(input.output)), {
      task_id: input.taskId,
      result_set_id: input.resultSetId,
      exec_unit_id: 'exec-01',
      spec_ref: input.specRef,
      plan_ref: input.planRef,
      required_methods: ['verification-before-completion'],
      source_capabilities: ['OpenSpec', 'superpowers']
    });
  } catch (error) {
    const message = (error as Error).message;
    if (message.includes('review report task_id 不一致')) {
      throw new Error(`review_report_task_mismatch: ${input.taskId}: ${message}`);
    }
    if (message.includes('review report result_set_id 不一致')) {
      throw new Error(`review_report_result_set_mismatch: ${input.taskId}: ${message}`);
    }
    throw new Error(
      `review_report_invalid: ${input.taskId}: ${message}; output_preview=${JSON.stringify(input.output.slice(0, 160))}`
    );
  }
}

function parseTestReport(input: {
  taskId: string;
  resultSetId: string;
  specRef: string;
  planRef: string;
  output: string;
}): TestReportArtifact {
  try {
    return validateTestReport(testReportSchema.parse(parseYaml(input.output)), {
      task_id: input.taskId,
      result_set_id: input.resultSetId,
      exec_unit_id: 'exec-01',
      spec_ref: input.specRef,
      plan_ref: input.planRef,
      required_methods: ['test-driven-development', 'verification-before-completion'],
      source_capabilities: ['OpenSpec', 'superpowers']
    });
  } catch (error) {
    const message = (error as Error).message;
    if (message.includes('test report task_id 不一致')) {
      throw new Error(`test_report_task_mismatch: ${input.taskId}: ${message}`);
    }
    if (message.includes('test report result_set_id 不一致')) {
      throw new Error(`test_report_result_set_mismatch: ${input.taskId}: ${message}`);
    }
    throw new Error(
      `test_report_invalid: ${input.taskId}: ${message}; output_preview=${JSON.stringify(input.output.slice(0, 160))}`
    );
  }
}

async function resolveClaudeStructuredOutput(
  output: string,
  stderr: string,
  fallbackPath: string,
  executionCwd: string
): Promise<string> {
  if (output.trim()) {
    return output;
  }

  const fallbackCandidates = [fallbackPath, path.join(executionCwd, fallbackPath)].map(candidate =>
    resolveWorkspacePath(candidate, executionCwd)
  );
  for (const candidate of fallbackCandidates) {
    try {
      const persisted = await fs.readFile(candidate, 'utf8');
      if (persisted.trim()) {
        return persisted;
      }
    } catch {
      continue;
    }
  }

  if (stderr.trim()) {
    return stderr;
  }

  return '';
}

export async function runReviewAndTest(taskId: string): Promise<{
  reviewReportPath: string;
  testReportPath: string;
}> {
  const state = await readState(taskId);
  const resultSetId = state.active_result_set_id ?? `result-set-${taskId}-01`;
  const artifactsDir = getTaskArtifactsDir(taskId);
  const reviewPromptPath = path.join(artifactsDir, 'review-prompt.md');
  const testPromptPath = path.join(artifactsDir, 'test-prompt.md');
  const reviewReportPath = path.join(artifactsDir, 'review-report.yaml');
  const testReportPath = path.join(artifactsDir, 'test-report.yaml');
  const executionCwd = state.context_bundle_ref
    ? validateExecutionContextBundle(
        parseYaml(await fs.readFile(resolveWorkspacePath(state.context_bundle_ref), 'utf8'))
      ).workspace_context.repo_path
    : process.cwd();
  const specRef = state.approved_spec_ref;
  const planRef = state.approved_plan_ref;

  if (!specRef || !planRef) {
    throw new Error(`缺少 review/test 所需的冻结引用: ${taskId}`);
  }

  await fs.mkdir(artifactsDir, { recursive: true });
  await fs.writeFile(reviewPromptPath, buildReviewPrompt(taskId, resultSetId), 'utf8');
  await fs.writeFile(testPromptPath, buildTestPrompt(taskId, resultSetId), 'utf8');

  const reviewRun = await runClaudeCode({
    cwd: executionCwd,
    promptPath: reviewPromptPath
  });
  if (reviewRun.exitCode !== 0) {
    throw new Error(`claude_review_failed: ${reviewRun.stderr}`);
  }
  const reviewOutput = await resolveClaudeStructuredOutput(reviewRun.stdout, reviewRun.stderr, reviewReportPath, executionCwd);
  const reviewReport = parseReviewReport({
    taskId,
    resultSetId,
    specRef,
    planRef,
    output: reviewOutput
  });

  const testRun = await runClaudeCode({
    cwd: executionCwd,
    promptPath: testPromptPath
  });
  if (testRun.exitCode !== 0) {
    throw new Error(`claude_test_failed: ${testRun.stderr}`);
  }
  const testOutput = await resolveClaudeStructuredOutput(testRun.stdout, testRun.stderr, testReportPath, executionCwd);
  const testReport = parseTestReport({
    taskId,
    resultSetId,
    specRef,
    planRef,
    output: testOutput
  });

  await fs.writeFile(reviewReportPath, stringifyYaml(reviewReport), 'utf8');
  await fs.writeFile(testReportPath, stringifyYaml(testReport), 'utf8');

  await fs.access(reviewReportPath);
  await fs.access(testReportPath);

  return { reviewReportPath, testReportPath };
}
