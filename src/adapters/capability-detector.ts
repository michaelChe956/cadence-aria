import { getHostCapability } from './host/host-adapter.js';
import { getOpenSpecCapability } from './openspec/openspec-adapter.js';
import { getSuperpowersCapability } from './superpowers/superpowers-adapter.js';

export function detectCapabilities() {
  return {
    host: getHostCapability(),
    openspec: getOpenSpecCapability(),
    superpowers: getSuperpowersCapability(),
    codex: {
      available: true,
      source: 'local-agent'
    }
  };
}
