import { accessSync, constants } from 'node:fs';
import path from 'node:path';

import { getHostCapability } from './host/host-adapter.js';
import { getOpenSpecCapability } from './openspec/openspec-adapter.js';
import { getSuperpowersCapability } from './superpowers/superpowers-adapter.js';

function getLauncherSuffixes(): string[] {
  if (process.platform === 'win32') {
    const pathext = process.env.PATHEXT?.split(';').map(value => value.trim()).filter(Boolean) ?? [];
    if (pathext.length > 0) {
      return pathext.map(value => (value.startsWith('.') ? value : `.${value}`));
    }

    return ['.exe', '.cmd', '.bat', '.ps1'];
  }

  return ['', '.cmd', '.bat'];
}

function getBinaryCandidates(binaryName: string): string[] {
  return getLauncherSuffixes().map(suffix => `${binaryName}${suffix}`);
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

function detectCli(binaryPath: string | null, binaryName: string) {
  if (!binaryPath) {
    return { available: false, source: binaryName };
  }

  try {
    accessSync(binaryPath, constants.X_OK);
    return { available: true, source: binaryPath };
  } catch {
    return { available: false, source: binaryName };
  }
}

export function detectCapabilities() {
  const codexBinaryPath = resolveBinaryPath('codex');

  return {
    host: getHostCapability(),
    openspec: getOpenSpecCapability(),
    superpowers: getSuperpowersCapability(),
    codex: detectCli(codexBinaryPath, 'codex')
  };
}
