import fs from 'node:fs/promises';
import path from 'node:path';

import { parseState, type State } from '../../schemas/state-schema.js';
import { getTaskStatePath } from './paths.js';
import { parseYaml, stringifyYaml } from '../../utils/yaml.js';

export async function readState(taskId: string): Promise<State> {
  const raw = await fs.readFile(getTaskStatePath(taskId), 'utf8');
  return parseState(parseYaml(raw));
}

export async function writeState(state: State): Promise<void> {
  const statePath = getTaskStatePath(state.task_id);
  await fs.mkdir(path.dirname(statePath), { recursive: true });
  await fs.writeFile(statePath, stringifyYaml(state), 'utf8');
}
