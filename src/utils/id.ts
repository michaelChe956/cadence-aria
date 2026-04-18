import fs from 'node:fs/promises';
import path from 'node:path';

const TASK_ROOT = path.posix.join('cadence', 'cache', 'aria', 'tasks');
const TASK_ID_PATTERN = /^aria-(\d{8})-(\d{3})$/;

function formatDate(now: Date): string {
  return now.toISOString().slice(0, 10).replaceAll('-', '');
}

export async function createTaskId(now = new Date(), minimumSuffix = 1): Promise<string> {
  const date = formatDate(now);
  const prefix = `aria-${date}-`;
  const taskRoot = TASK_ROOT;

  let maxSuffix = minimumSuffix - 1;

  try {
    const entries = await fs.readdir(taskRoot, { withFileTypes: true });
    for (const entry of entries) {
      if (!entry.isDirectory()) {
        continue;
      }

      const match = entry.name.match(TASK_ID_PATTERN);
      if (!match || match[1] !== date) {
        continue;
      }

      const suffix = Number(match[2]);
      if (suffix >= minimumSuffix && suffix > maxSuffix) {
        maxSuffix = suffix;
      }
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw error;
    }
  }

  const nextSuffix = String(maxSuffix + 1).padStart(3, '0');
  return `${prefix}${nextSuffix}`;
}
