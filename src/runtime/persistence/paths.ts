import path from 'node:path';

const TASK_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]*$/;

function assertValidTaskId(taskId: string): void {
  if (!TASK_ID_PATTERN.test(taskId)) {
    throw new Error(`非法 taskId: ${taskId}`);
  }
}

export function getTaskRoot(taskId: string): string {
  assertValidTaskId(taskId);
  return path.posix.join('cadence', 'cache', 'aria', 'tasks', taskId);
}

export function getTaskStatePath(taskId: string): string {
  return path.posix.join(getTaskRoot(taskId), 'state.yaml');
}

export function getTaskArtifactsDir(taskId: string): string {
  return path.posix.join(getTaskRoot(taskId), 'artifacts');
}
