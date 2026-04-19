import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { doctorCommand } from '../../../src/commands/doctor.js';

const ORIGINAL_PATH = process.env.PATH ?? '';

let fakeBinDir = '';

async function createFakeBinaries(): Promise<void> {
  fakeBinDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-doctor-bins-'));

  for (const binary of ['claude', 'codex']) {
    const scriptPath = path.join(fakeBinDir, binary);
    await fs.writeFile(scriptPath, '#!/bin/sh\nexit 0\n', 'utf8');
    await fs.chmod(scriptPath, 0o755);
  }

  process.env.PATH = `${fakeBinDir}${path.delimiter}${ORIGINAL_PATH}`;
}

beforeEach(async () => {
  await createFakeBinaries();
});

afterEach(async () => {
  process.env.PATH = ORIGINAL_PATH;
  if (fakeBinDir) {
    await fs.rm(fakeBinDir, { recursive: true, force: true });
    fakeBinDir = '';
  }
});

describe('doctor command', () => {
  it('返回 claude code、codex、OpenSpec、superpowers 的真实可用状态', async () => {
    const output = await doctorCommand();

    expect(output).toContain('claude_code');
    expect(output).toContain('codex');
    expect(output).toContain('OpenSpec');
    expect(output).toContain('superpowers');
    expect(output).toContain('- available: true');
  });
});
