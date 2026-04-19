import { accessSync, constants } from 'node:fs';
import path from 'node:path';

import { getHostCapability } from './host/host-adapter.js';
import { getOpenSpecCapability } from './openspec/openspec-adapter.js';
import { getSuperpowersCapability } from './superpowers/superpowers-adapter.js';

function getBinaryCandidates(binaryName: string): string[] {
  const launcherSuffixes = process.platform === 'win32'
    ? ['.exe', '.cmd', '.bat', '.ps1', '']
    : ['', '.cmd', '.bat'];

  return launcherSuffixes.map(suffix => `${binaryName}${suffix}`);
}

function resolveBinaryPath(binaryName: string): string | null {
  const pathEntries = process.env.PATH?.split(path.delimiter) ?? [];
  const candidates = getBinaryCandidates(binaryName);

  for (const entry of pathEntries) {
    if (!entry) {
      continue;
    }

    for (const candidateName of candidates) {
      const candidate = path.join(entry, candidateName);
      try {
        accessSync(candidate, constants.X_OK);
        return candidate;
      } catch {
        continue;
      }
    }
  }

  return null;
}

function detectCli(binaryPath: string) {
  try {
    accessSync(binaryPath, constants.X_OK);
    return { available: true, source: binaryPath };
  } catch {
    return { available: false, source: binaryPath };
  }
}

export function detectCapabilities() {
  const codexBinaryPath = resolveBinaryPath('codex') ?? 'codex';

  return {
    host: getHostCapability(),
    openspec: getOpenSpecCapability(),
    superpowers: getSuperpowersCapability(),
    codex: detectCli(codexBinaryPath)
  };
}
