export function buildSpecArtifact(title: string): string {
  return `# Spec\n\ngoal: ${title}\nscope: 仅覆盖一期 formal flow 最小闭环\n`;
}
