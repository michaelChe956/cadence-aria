import { existsSync } from 'node:fs';
import path from 'node:path';

function hasWorkspaceMarker(dir: string): boolean {
  return (
    existsSync(path.join(dir, 'package.json')) ||
    existsSync(path.join(dir, '.git')) ||
    existsSync(path.join(dir, 'cadence'))
  );
}

export function findWorkspaceRoot(start = process.cwd()): string {
  let current = path.resolve(start);

  while (true) {
    if (hasWorkspaceMarker(current)) {
      return current;
    }

    const parent = path.dirname(current);
    if (parent === current) {
      return path.resolve(start);
    }
    current = parent;
  }
}

export function resolveWorkspacePath(targetPath: string, workspaceRoot = findWorkspaceRoot()): string {
  if (path.isAbsolute(targetPath)) {
    return targetPath;
  }

  return path.join(workspaceRoot, targetPath);
}
