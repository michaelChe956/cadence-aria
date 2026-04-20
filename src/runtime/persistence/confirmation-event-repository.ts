import fs from 'node:fs/promises';
import os from 'node:os';
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

  let existingEvents: unknown[] = [];
  try {
    const content = await fs.readFile(target, 'utf8');
    const parsed = parseYaml(content);
    existingEvents = Array.isArray(parsed) ? parsed : [];

    if (existingEvents.some(existing => matchesConfirmationEvent(existing, event))) {
      return target;
    }
  } catch {
    // no existing confirmation log yet
  }

  const updatedEvents = [...existingEvents, event];
  const tmpFile = `${target}.${process.pid}-${Date.now()}.tmp`;
  await fs.writeFile(tmpFile, stringifyYaml(updatedEvents), 'utf8');
  await fs.rename(tmpFile, target);
  return target;
}
