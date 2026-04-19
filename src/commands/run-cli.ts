import { cancelCommand } from './cancel.js';
import { confirmPlanCommand } from './confirm-plan.js';
import { confirmSpecCommand } from './confirm-spec.js';
import { doctorCommand } from './doctor.js';
import { intakeCommand } from './intake.js';
import { resultCommand } from './result.js';
import { retryCommand } from './retry.js';
import { runCommand } from './run.js';
import { startCommand } from './start.js';
import { statusCommand } from './status.js';

const HELP_TEXT = [
  'aria:intake',
  'aria:start',
  'confirm-spec',
  'confirm-plan',
  'aria:run',
  'aria:status',
  'aria:result',
].join('\n');

export async function runCli(args: string[]): Promise<string> {
  const [command, ...rest] = args;

  if (!command || command === '--help' || command === '-h') {
    return HELP_TEXT;
  }

  if (command === 'aria:intake' || command === 'intake') {
    const title = rest.join(' ').trim();
    if (!title) {
      throw new Error('aria:intake 需要标题');
    }
    return intakeCommand(title);
  }

  if (command === 'aria:start' || command === 'start') {
    const taskId = readOption(rest, '--task-id');
    return startCommand(taskId);
  }

  if (command === 'confirm-spec') {
    const taskId = readOption(rest, '--task-id');
    return confirmSpecCommand(taskId);
  }

  if (command === 'confirm-plan') {
    const taskId = readOption(rest, '--task-id');
    return confirmPlanCommand(taskId);
  }

  if (command === 'aria:run' || command === 'run') {
    const taskId = readOption(rest, '--task-id');
    return runCommand(taskId);
  }

  if (command === 'aria:status' || command === 'status') {
    const taskId = readOption(rest, '--task-id');
    return statusCommand(taskId);
  }

  if (command === 'aria:result' || command === 'result') {
    const taskId = readOption(rest, '--task-id');
    return resultCommand(taskId);
  }

  if (command === 'aria:cancel' || command === 'cancel') {
    const taskId = readOption(rest, '--task-id');
    return cancelCommand(taskId);
  }

  if (command === 'aria:retry' || command === 'retry') {
    const taskId = readOption(rest, '--task-id');
    return retryCommand(taskId);
  }

  if (command === 'aria:doctor' || command === 'doctor') {
    return doctorCommand();
  }

  throw new Error(`未知命令: ${command}`);
}

function readOption(args: string[], name: string): string {
  const index = args.indexOf(name);
  if (index === -1 || index === args.length - 1) {
    throw new Error(`缺少参数: ${name}`);
  }

  const value = args[index + 1]?.trim();
  if (!value) {
    throw new Error(`缺少参数值: ${name}`);
  }

  return value;
}
