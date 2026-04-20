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
import { validateExecutionContextBundle } from '../contracts/contract-validator.js';

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

function validateReviewReportContext(input: {
  taskId: string;
  resultSetId: string;
  report: ReviewReportArtifact;
}): ReviewReportArtifact {
  if (input.report.task_id !== input.taskId) {
    throw new Error(`review_report_task_mismatch: ${input.taskId}: actual=${input.report.task_id}`);
  }

  if (input.report.result_set_id !== input.resultSetId) {
    throw new Error(`review_report_result_set_mismatch: ${input.taskId}: actual=${input.report.result_set_id}`);
  }

  return input.report;
}

function validateTestReportContext(input: {
  taskId: string;
  resultSetId: string;
  report: TestReportArtifact;
}): TestReportArtifact {
  if (input.report.task_id !== input.taskId) {
    throw new Error(`test_report_task_mismatch: ${input.taskId}: actual=${input.report.task_id}`);
  }

  if (input.report.result_set_id !== input.resultSetId) {
    throw new Error(`test_report_result_set_mismatch: ${input.taskId}: actual=${input.report.result_set_id}`);
  }

  return input.report;
}

function parseReviewReport(taskId: string, resultSetId: string, output: string): ReviewReportArtifact {
  try {
    return validateReviewReportContext({
      taskId,
      resultSetId,
      report: reviewReportSchema.parse(parseYaml(output))
    });
  } catch (error) {
    throw new Error(
      `review_report_invalid: ${taskId}: ${(error as Error).message}; output_preview=${JSON.stringify(output.slice(0, 160))}`
    );
  }
}

function parseTestReport(taskId: string, resultSetId: string, output: string): TestReportArtifact {
  try {
    return validateTestReportContext({
      taskId,
      resultSetId,
      report: testReportSchema.parse(parseYaml(output))
    });
  } catch (error) {
    throw new Error(
      `test_report_invalid: ${taskId}: ${(error as Error).message}; output_preview=${JSON.stringify(output.slice(0, 160))}`
    );
  }
}

async function resolveClaudeStructuredOutput(
  output: string,
  stderr: string,
  fallbackPath: string
): Promise<string> {
  if (output.trim()) {
    return output;
  }

  if (stderr.trim()) {
    return stderr;
  }

  try {
    const persisted = await fs.readFile(fallbackPath, 'utf8');
    if (persisted.trim()) {
      return persisted;
    }
  } catch {
    // ignore missing fallback artifact
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
        parseYaml(await fs.readFile(state.context_bundle_ref, 'utf8'))
      ).workspace_context.repo_path
    : process.cwd();

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
  const reviewOutput = await resolveClaudeStructuredOutput(reviewRun.stdout, reviewRun.stderr, reviewReportPath);
  const reviewReport = parseReviewReport(taskId, resultSetId, reviewOutput);

  const testRun = await runClaudeCode({
    cwd: executionCwd,
    promptPath: testPromptPath
  });
  if (testRun.exitCode !== 0) {
    throw new Error(`claude_test_failed: ${testRun.stderr}`);
  }
  const testOutput = await resolveClaudeStructuredOutput(testRun.stdout, testRun.stderr, testReportPath);
  const testReport = parseTestReport(taskId, resultSetId, testOutput);

  await fs.writeFile(reviewReportPath, stringifyYaml(reviewReport), 'utf8');
  await fs.writeFile(testReportPath, stringifyYaml(testReport), 'utf8');

  await fs.access(reviewReportPath);
  await fs.access(testReportPath);

  return { reviewReportPath, testReportPath };
}
