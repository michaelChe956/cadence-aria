import { describe, expect, it } from 'vitest';
import { buildReviewReport } from '../../../../src/runtime/reports/review-report.js';

describe('buildReviewReport', () => {
  it('生成包含必要字段的 review report', () => {
    const report = buildReviewReport('task-001', 'rs-001');
    expect(report.task_id).toBe('task-001');
    expect(report.result_set_id).toBe('rs-001');
    expect(report.verdict).toBe('passed');
    expect(report.producer).toBe('claude-code');
    expect(report.blockers).toEqual([]);
    expect(report.suggestions).toEqual([]);
  });

  it('生成包含 baseline_refs 和 method_refs 的报告', () => {
    const report = buildReviewReport('task-002', 'rs-002');
    expect(report.baseline_refs.length).toBeGreaterThan(0);
    expect(report.method_refs.length).toBeGreaterThan(0);
  });

  it('包含 source_capabilities', () => {
    const report = buildReviewReport('task-003', 'rs-003');
    expect(report.source_capabilities).toContain('OpenSpec');
    expect(report.source_capabilities).toContain('superpowers');
  });

  it('generated_at 是 ISO 8601 格式', () => {
    const report = buildReviewReport('task-004', 'rs-004');
    expect(report.generated_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/);
  });
});
