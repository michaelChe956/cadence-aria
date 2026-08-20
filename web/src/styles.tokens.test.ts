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
