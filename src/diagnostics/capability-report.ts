import { detectCapabilities } from '../adapters/capability-detector.js';

export function buildCapabilityReport(): string {
  const capabilities = detectCapabilities();

  return [
    'claude_code',
    `- available: ${capabilities.claude_code.available}`,
    `- source: ${capabilities.claude_code.source}`,
    'codex',
    `- available: ${capabilities.codex.available}`,
    `- source: ${capabilities.codex.source}`,
    'OpenSpec',
    `- available: ${capabilities.openspec.available}`,
    `- source: ${capabilities.openspec.source}`,
    'superpowers',
    `- available: ${capabilities.superpowers.available}`,
    `- source: ${capabilities.superpowers.source}`
  ].join('\n');
}
