import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { beforeEach, afterEach, describe, expect, it } from 'vitest';

import { appendConfirmationEvent } from '../../../src/runtime/persistence/confirmation-event-repository.js';
import { createTask } from '../../../src/runtime/persistence/task-repository.js';
import { readState } from '../../../src/runtime/persistence/state-repository.js';

const ORIGINAL_CWD = process.cwd();

let tempDir = '';

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-state-repository-'));
  process.chdir(tempDir);
}

async function restoreWorkspace(): Promise<void> {
  process.chdir(ORIGINAL_CWD);
  if (tempDir) {
    await fs.rm(tempDir, { recursive: true, force: true });
  }
}

beforeEach(async () => {
  await setTempWorkspace();
});

afterEach(async () => {
  await restoreWorkspace();
});

describe('createTask', () => {
  it('会落盘 state.yaml 并可回读解析', async () => {
    const task = await createTask({
      title: '为 Aria 增加 capability report 结构化输出',
    });

    await expect(fs.access(path.join('cadence', 'cache', 'aria', 'tasks', task.task_id, 'state.yaml'))).resolves.toBeUndefined();

    const state = await readState(task.task_id);

    expect(task.task_id).toMatch(/^aria-\d{8}-\d{3}$/);
    expect(state.task_id).toBe(task.task_id);
    expect(task.status).toBe('intake');
    expect(task.source).toBe('aria-native');
    expect(state.status).toBe('intake');
  });

  it('会写入 confirmation-events.yaml', async () => {
    const task = await createTask({
      title: '为 Aria 增加 capability report 结构化输出',
    });

    const confirmationPath = await appendConfirmationEvent(task.task_id, {
      event_type: 'spec-confirmed',
      confirmed_at: '2026-04-18T00:00:00.000Z',
    });

    const content = await fs.readFile(confirmationPath, 'utf8');

    expect(content).toContain('spec-confirmed');
    expect(content).toContain('confirmed_at');
  });

  it('连续创建两个任务时 task_id 不重复', async () => {
    const first = await createTask({
      title: '任务一',
    });
    const second = await createTask({
      title: '任务二',
    });

    expect(first.task_id).not.toBe(second.task_id);
    expect(first.task_id).toMatch(/^aria-\d{8}-\d{3}$/);
    expect(second.task_id).toMatch(/^aria-\d{8}-\d{3}$/);
  });
});
