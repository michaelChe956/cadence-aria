import { detectCapabilities } from '../adapters/capability-detector.js';

export function buildCapabilityReport(): string {
  const capabilities = detectCapabilities();

  return [
    'OpenSpec',
    `- available: ${capabilities.openspec.available}`,
    `- source: ${capabilities.openspec.source}`,
    'superpowers',
    `- available: ${capabilities.superpowers.available}`,
    `- source: ${capabilities.superpowers.source}`,
    'Codex',
    `- available: ${capabilities.codex.available}`,
    `- source: ${capabilities.codex.source}`
  ].join('\n');
}
