import { describe, expect, it } from 'vitest';
import { createTask } from '../../../src/runtime/persistence/task-repository.js';

describe('createTask', () => {
  it('初始化任务目录并写入 intake 状态', async () => {
    const task = await createTask({
      title: '为 Aria 增加 capability report 结构化输出',
    });

    expect(task.task_id).toMatch(/^aria-20260418-/);
    expect(task.status).toBe('intake');
    expect(task.source).toBe('aria-native');
  });
});
