const HELP_TEXT = [
  'aria:intake',
  'aria:start',
  'aria:run',
  'aria:status',
  'aria:result',
].join('\n');

export async function runCli(args: string[]): Promise<string> {
  return HELP_TEXT;
}
