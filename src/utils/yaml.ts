import { parse as parseYamlDocument, stringify as stringifyYamlDocument } from 'yaml';

export function parseYaml(input: string): unknown {
  return parseYamlDocument(input);
}

export function stringifyYaml(value: unknown): string {
  return stringifyYamlDocument(value);
}
