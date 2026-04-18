import { describe, expect, it } from 'vitest';
import { getTaskArtifactsDir, getTaskRoot, getTaskStatePath } from '../../../../src/runtime/persistence/paths.js';

describe('任务路径工具', () => {
  it('会拼接合法 taskId 的任务路径', () => {
    expect(getTaskRoot('aria-20260418-001')).toBe('cadence/cache/aria/tasks/aria-20260418-001');
    expect(getTaskStatePath('aria-20260418-001')).toBe('cadence/cache/aria/tasks/aria-20260418-001/state.yaml');
    expect(getTaskArtifactsDir('aria-20260418-001')).toBe('cadence/cache/aria/tasks/aria-20260418-001/artifacts');
  });

  it.each(['', '../escape', '/abs', 'foo/../bar', 'foo//bar', 'foo bar'])('拒绝非法 taskId: %s', (taskId) => {
    expect(() => getTaskRoot(taskId)).toThrowError();
    expect(() => getTaskStatePath(taskId)).toThrowError();
    expect(() => getTaskArtifactsDir(taskId)).toThrowError();
  });
});
