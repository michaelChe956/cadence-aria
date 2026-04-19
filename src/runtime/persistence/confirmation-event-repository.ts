import fs from 'node:fs/promises';
import path from 'node:path';

import { parseYaml, stringifyYaml } from '../../utils/yaml.js';
import { getTaskRoot } from './paths.js';

function matchesConfirmationEvent(existing: unknown, event: Record<string, string>): boolean {
  if (!existing || typeof existing !== 'object' || Array.isArray(existing)) {
    return false;
  }

  const candidate = existing as Record<string, unknown>;
  return candidate.task_id === event.task_id
    && candidate.confirmation_type === event.confirmation_type
    && candidate.artifact_ref === event.artifact_ref
    && candidate.decision === event.decision;
}

export async function appendConfirmationEvent(taskId: string, event: Record<string, string>): Promise<string> {
  const target = path.join(getTaskRoot(taskId), 'confirmation-events.yaml');
  await fs.mkdir(path.dirname(target), { recursive: true });

  try {
    const content = await fs.readFile(target, 'utf8');
    const parsed = parseYaml(content);
    const existingEvents = Array.isArray(parsed) ? parsed : [];

    if (existingEvents.some(existing => matchesConfirmationEvent(existing, event))) {
      return target;
    }
  } catch {
    // no existing confirmation log yet
  }

  await fs.appendFile(target, `${stringifyYaml([event])}\n`, 'utf8');
  return target;
}
