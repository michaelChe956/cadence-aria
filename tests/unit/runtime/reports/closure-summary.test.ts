import { describe, expect, it } from 'vitest';
import { buildClosureSummary } from '../../../../src/runtime/reports/closure-summary.js';

describe('buildClosureSummary', () => {
  it('生成包含 task_id 的 closure summary', () => {
    const summary = buildClosureSummary('task-001');
    expect(summary).toContain('task_id: task-001');
  });

  it('包含 final_status: done', () => {
    const summary = buildClosureSummary('task-001');
    expect(summary).toContain('final_status: done');
  });

  it('使用不同的 taskId 生成不同结果', () => {
    const a = buildClosureSummary('task-a');
    const b = buildClosureSummary('task-b');
    expect(a).not.toBe(b);
    expect(a).toContain('task-a');
    expect(b).toContain('task-b');
  });
});
