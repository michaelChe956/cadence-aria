import { describe, expect, it } from 'vitest';
import { buildPatchContract } from '../../../../src/runtime/contracts/patch-contract.js';

describe('buildPatchContract', () => {
  it('生成包含 task_id 的 patch contract', () => {
    const contract = buildPatchContract('task-001', 'rs-001');
    expect(contract.task_id).toBe('task-001');
  });

  it('包含 based_on_result_set_id', () => {
    const contract = buildPatchContract('task-001', 'rs-001');
    expect(contract.based_on_result_set_id).toBe('rs-001');
  });

  it('contract_type 为 patch', () => {
    const contract = buildPatchContract('task-001', 'rs-001');
    expect(contract.contract_type).toBe('patch');
  });

  it('包含 must_fix_items', () => {
    const contract = buildPatchContract('task-001', 'rs-001');
    expect(Array.isArray(contract.must_fix_items)).toBe(true);
    expect(contract.must_fix_items.length).toBeGreaterThan(0);
  });

  it('generated_at 是 ISO 8601 格式', () => {
    const contract = buildPatchContract('task-001', 'rs-001');
    expect(contract.generated_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/);
  });

  it('包含 based_on_spec_ref 和 based_on_plan_ref', () => {
    const contract = buildPatchContract('task-001', 'rs-001');
    expect(contract.based_on_spec_ref).toBeTruthy();
    expect(contract.based_on_plan_ref).toBeTruthy();
  });
});
