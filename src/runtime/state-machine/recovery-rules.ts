export function resolveRetryableBlock(reason: string) {
  if (reason === 'execution_blocked' || reason === 'capability_blocked') {
    return { retryable: true, required_action: '修复执行条件后重新运行 aria:retry' };
  }

  if (reason === 'input_blocked') {
    return { retryable: false, required_action: '补齐缺失的输入工件后重新运行' };
  }

  if (reason === 'decision_blocked') {
    return { retryable: false, required_action: '等待人工决策后重新确认' };
  }

  return { retryable: false, required_action: '人工处理并补齐合法工件' };
}
