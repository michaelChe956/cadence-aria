import fs from 'node:fs/promises';
import path from 'node:path';

import { stringifyYaml } from '../../utils/yaml.js';
import { getTaskRoot } from './paths.js';

export async function appendConfirmationEvent(taskId: string, event: Record<string, string>): Promise<string> {
  const target = path.join(getTaskRoot(taskId), 'confirmation-events.yaml');
  await fs.mkdir(path.dirname(target), { recursive: true });
  await fs.appendFile(target, `${stringifyYaml([event])}\n`, 'utf8');
  return target;
}
