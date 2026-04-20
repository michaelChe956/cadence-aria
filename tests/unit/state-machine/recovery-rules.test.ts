import { describe, expect, it } from 'vitest';
import { resolveRetryableBlock } from '../../../src/runtime/state-machine/recovery-rules.js';

describe('resolveRetryableBlock', () => {
  it('execution_blocked 是可重试的', () => {
    const result = resolveRetryableBlock('execution_blocked');
    expect(result.retryable).toBe(true);
    expect(result.required_action).toContain('aria:retry');
  });

  it('capability_blocked 是可重试的', () => {
    const result = resolveRetryableBlock('capability_blocked');
    expect(result.retryable).toBe(true);
  });

  it('input_blocked 不可重试', () => {
    const result = resolveRetryableBlock('input_blocked');
    expect(result.retryable).toBe(false);
    expect(result.required_action).toContain('输入工件');
  });

  it('decision_blocked 不可重试', () => {
    const result = resolveRetryableBlock('decision_blocked');
    expect(result.retryable).toBe(false);
    expect(result.required_action).toContain('人工决策');
  });

  it('未知原因默认不可重试', () => {
    const result = resolveRetryableBlock('unknown_reason');
    expect(result.retryable).toBe(false);
    expect(result.required_action).toContain('人工处理');
  });
});
