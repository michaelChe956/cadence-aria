import { parse as parseYamlDocument, stringify as stringifyYamlDocument } from 'yaml';

export function parseYaml<T>(input: string): T {
  return parseYamlDocument(input) as T;
}

export function stringifyYaml(value: unknown): string {
  return stringifyYamlDocument(value);
}
