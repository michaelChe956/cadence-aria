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

  return [''];
}

function getBinaryCandidates(binaryName: string): string[] {
  return getLauncherSuffixes().map(suffix => `${binaryName}${suffix}`);
}

function resolveBinaryPath(binaryName: string): string | null {
  const pathEntries = process.env.PATH?.split(path.delimiter) ?? [];
  const candidates = getBinaryCandidates(binaryName);

  for (const entry of pathEntries) {
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

function resolveConfiguredBinaryPath(envName: 'CADENCE_CLAUDE_BIN' | 'CADENCE_CODEX_BIN'): string | null {
  const configuredPath = process.env[envName]?.trim();
  if (!configuredPath) {
    return null;
  }

  return configuredPath;
}

export function detectCapabilities() {
  const claudeBinaryPath = resolveConfiguredBinaryPath('CADENCE_CLAUDE_BIN') ?? resolveBinaryPath('claude');
  const codexBinaryPath = resolveConfiguredBinaryPath('CADENCE_CODEX_BIN') ?? resolveBinaryPath('codex');

  return {
    host: getHostCapability(),
    claude_code: detectCli(claudeBinaryPath, 'claude'),
    openspec: getOpenSpecCapability(),
    superpowers: getSuperpowersCapability(),
    codex: detectCli(codexBinaryPath, 'codex')
  };
}
