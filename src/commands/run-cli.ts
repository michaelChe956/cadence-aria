const HELP_TEXT = [
  'aria:intake',
  'aria:start',
  'aria:run',
  'aria:status',
  'aria:result',
].join('\n');

export async function runCli(args: string[]): Promise<string> {
  if (args.length === 0) {
    return HELP_TEXT;
  }

  return HELP_TEXT;
}
