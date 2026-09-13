// @ts-expect-error - web tsconfig 无 @types/node，vitest 运行时 node:fs 可用
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("aria tokens follow canvas-experience palette", () => {
  const css = readFileSync("src/styles.css", "utf8");
  expect(css).toContain("--aria-primary: #4F46E5");
  expect(css).toContain("--aria-cta: #F97316");
  expect(css).toContain("--aria-bg: #f5f3ee");
});

it("aria token set includes soft variants and strong border", () => {
  const css = readFileSync("src/styles.css", "utf8");
  expect(css).toContain("--aria-primary-soft: #e0e7ff");
  expect(css).toContain("--aria-cta-soft: #ffedd5");
  expect(css).toContain("--aria-border-strong: #3f3f46");
});

it("tailwind config exposes 3px border width and canvas palette", () => {
  const config = readFileSync("tailwind.config.ts", "utf8");
  expect(config).toContain('3: "3px"');
  expect(config).toContain("#4F46E5");
  expect(config).toContain("#F97316");
});

it("adds the three derived token groups from existing semantic colors", () => {
  const css = readFileSync("src/styles.css", "utf8");
  for (const token of [
    "--aria-conv-user-bg",
    "--aria-conv-user-ink",
    "--aria-conv-agent-bg",
    "--aria-conv-agent-ink",
    "--aria-conv-turn-rail",
    "--aria-conv-turn-active",
    "--aria-conv-caret",
  ]) {
    expect(css).toContain(token);
  }
  for (const state of ["running", "pending", "blocked", "done", "failed", "awaiting-triage"]) {
    expect(css).toContain(`--aria-topo-node-${state}-fg`);
    expect(css).toContain(`--aria-topo-node-${state}-bg`);
    expect(css).toContain(`--aria-topo-node-${state}-border`);
  }
  expect(css).toContain("--aria-topo-edge:");
  expect(css).toContain("--aria-topo-edge-active:");
  for (const state of ["open", "pending", "passed"]) {
    expect(css).toContain(`--aria-gate-${state}-fg`);
    expect(css).toContain(`--aria-gate-${state}-bg`);
    expect(css).toContain(`--aria-gate-${state}-border`);
  }
});

it("keeps the reduced-motion baseline and never bypasses it for new animation", () => {
  const css = readFileSync("src/styles.css", "utf8");
  expect(css).toContain("animation-duration: 0.001ms !important");
  const start = css.indexOf(".aria-pulse {");
  expect(start).toBeGreaterThan(-1);
  const pulseRule = css.slice(start, css.indexOf("}", start));
  expect(pulseRule).not.toContain("!important");
  expect(css).toContain("@keyframes aria-cockpit-pulse");
});

it("exposes mono and tabular-nums helpers", () => {
  const css = readFileSync("src/styles.css", "utf8");
  expect(css).toContain("--aria-font-mono: ui-monospace");
  expect(css).toContain(".aria-mono");
  expect(css).toContain(".aria-num");
});
