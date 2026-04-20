import { describe, expect, it } from 'vitest';
import { buildVerificationSummary } from '../../../../src/runtime/reports/verification-summary.js';

describe('buildVerificationSummary', () => {
  it('生成包含 task_id 的 verification summary', () => {
    const summary = buildVerificationSummary('task-001');
    expect(summary).toContain('task_id: task-001');
  });

  it('包含 status: verified', () => {
    const summary = buildVerificationSummary('task-001');
    expect(summary).toContain('status: verified');
  });

  it('使用不同的 taskId 生成不同结果', () => {
    const a = buildVerificationSummary('task-a');
    const b = buildVerificationSummary('task-b');
    expect(a).not.toBe(b);
    expect(a).toContain('task-a');
    expect(b).toContain('task-b');
  });
});
