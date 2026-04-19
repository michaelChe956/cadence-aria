import fs from 'node:fs/promises';
import path from 'node:path';

import { runClaudeCode } from '../../adapters/claude-code/claude-code-adapter.js';
import { stringifyYaml } from '../../utils/yaml.js';
import { readState } from '../persistence/state-repository.js';
import { getTaskArtifactsDir } from '../persistence/paths.js';
import { buildReviewReport } from '../reports/review-report.js';
import { buildTestReport } from '../reports/test-report.js';

function buildReviewPrompt(taskId: string, resultSetId: string): string {
  return [
    '# Claude Code Review Prompt',
    `task_id: ${taskId}`,
    `result_set_id: ${resultSetId}`,
    '不要读取仓库规则与文件，不要执行命令，不要修改任何文件。',
    '不要生成 report 文件，也不要解释流程。',
    `只输出一行中文摘要：已接收 review 任务 ${taskId}。`
  ].join('\n');
}

function buildTestPrompt(taskId: string, resultSetId: string): string {
  return [
    '# Claude Code Test Prompt',
    `task_id: ${taskId}`,
    `result_set_id: ${resultSetId}`,
    '不要读取仓库规则与文件，不要执行命令，不要修改任何文件。',
    '不要生成 report 文件，也不要解释流程。',
    `只输出一行中文摘要：已接收 test 任务 ${taskId}。`
  ].join('\n');
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

  await fs.mkdir(artifactsDir, { recursive: true });
  await fs.writeFile(reviewPromptPath, buildReviewPrompt(taskId, resultSetId), 'utf8');
  await fs.writeFile(testPromptPath, buildTestPrompt(taskId, resultSetId), 'utf8');

  const reviewRun = await runClaudeCode({
    cwd: process.cwd(),
    promptPath: reviewPromptPath
  });
  if (reviewRun.exitCode !== 0) {
    throw new Error(`claude_review_failed: ${reviewRun.stderr}`);
  }

  const testRun = await runClaudeCode({
    cwd: process.cwd(),
    promptPath: testPromptPath
  });
  if (testRun.exitCode !== 0) {
    throw new Error(`claude_test_failed: ${testRun.stderr}`);
  }

  await fs.writeFile(reviewReportPath, stringifyYaml(buildReviewReport(taskId, resultSetId)), 'utf8');
  await fs.writeFile(testReportPath, stringifyYaml(buildTestReport(taskId, resultSetId)), 'utf8');

  await fs.access(reviewReportPath);
  await fs.access(testReportPath);

  return { reviewReportPath, testReportPath };
}
