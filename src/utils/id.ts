let counter = 1;

export function createTaskId(now = new Date('2026-04-18T00:00:00.000Z')): string {
  const date = now.toISOString().slice(0, 10).replaceAll('-', '');
  const suffix = String(counter++).padStart(3, '0');
  return `aria-${date}-${suffix}`;
}
