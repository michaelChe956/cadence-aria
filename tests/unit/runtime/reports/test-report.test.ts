import { describe, expect, it } from 'vitest';
import { buildTestReport } from '../../../../src/runtime/reports/test-report.js';

describe('buildTestReport', () => {
  it('生成包含必要字段的 test report', () => {
    const report = buildTestReport('task-001', 'rs-001');
    expect(report.task_id).toBe('task-001');
    expect(report.result_set_id).toBe('rs-001');
    expect(report.verdict).toBe('passed');
    expect(report.producer).toBe('claude-code');
  });

  it('默认通过测试', () => {
    const report = buildTestReport('task-001', 'rs-001');
    expect(report.passed_count).toBeGreaterThanOrEqual(0);
    expect(report.failed_count).toBe(0);
    expect(report.failures).toEqual([]);
  });

  it('包含 commands_run 和 method_refs', () => {
    const report = buildTestReport('task-001', 'rs-001');
    expect(report.commands_run.length).toBeGreaterThan(0);
    expect(report.method_refs.length).toBeGreaterThan(0);
  });

  it('包含 source_capabilities', () => {
    const report = buildTestReport('task-001', 'rs-001');
    expect(report.source_capabilities).toContain('OpenSpec');
    expect(report.source_capabilities).toContain('superpowers');
  });

  it('generated_at 是 ISO 8601 格式', () => {
    const report = buildTestReport('task-001', 'rs-001');
    expect(report.generated_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/);
  });
});
