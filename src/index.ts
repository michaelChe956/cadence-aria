import { runCli } from './commands/run-cli.js';

async function main(): Promise<void> {
  const output = await runCli(process.argv.slice(2));
  process.stdout.write(`${output}\n`);
}

void main().catch((error: unknown) => {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`[Aria] 错误: ${message}\n`);
  process.exit(1);
});
