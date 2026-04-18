import path from 'node:path';

export function getTaskRoot(taskId: string): string {
  return path.posix.join('cadence', 'cache', 'aria', 'tasks', taskId);
}

export function getTaskStatePath(taskId: string): string {
  return path.posix.join(getTaskRoot(taskId), 'state.yaml');
}

export function getTaskArtifactsDir(taskId: string): string {
  return getTaskRoot(taskId);
}
