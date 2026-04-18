import { describe, expect, it } from 'vitest';
import { parseState } from '../../../src/schemas/state-schema.js';

describe('parseState', () => {
  it('接受一期 formal flow 的最小状态对象', () => {
    const state = parseState({
      task_id: 'aria-20260418-001',
      task_title: '为 Aria 增加 capability report 结构化输出',
      source: 'aria-native',
      flow_type: 'formal',
      risk_level: 'medium',
      status: 'intake',
      current_round: 1,
      approved_spec_ref: null,
      approved_plan_ref: null,
      active_result_set_id: null,
      active_exec_units: [],
      confirmation_pending: 'none',
      confirmation_mode: 'manual',
      confirmation_artifact_path: null,
      review_status: 'pending',
      test_status: 'pending',
      patch_required_by: 'none',
      patch_round: 0,
      exec_units: {},
      created_at: '2026-04-18T00:00:00.000Z',
      updated_at: '2026-04-18T00:00:00.000Z'
    });

    expect(state.status).toBe('intake');
  });
});
