import { describe, expect, it } from 'vitest';
import { parseYaml, stringifyYaml } from '../../../src/utils/yaml.js';

describe('yaml 工具', () => {
  it('支持基础 round-trip', () => {
    const input = {
      task_id: 'aria-20260418-001',
      nested: {
        enabled: true,
        items: ['a', 'b']
      }
    };

    const yamlText = stringifyYaml(input);
    const parsed = parseYaml(yamlText);

    expect(parsed).toEqual(input);
  });
});
