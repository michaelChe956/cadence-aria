import { runCli } from './commands/run-cli.js';

async function main(): Promise<void> {
  const output = await runCli(process.argv.slice(2));
  process.stdout.write(`${output}\n`);
}

void main();
