export function resolveRetryableBlock(reason: string) {
  if (reason === 'execution_blocked') {
    return { retryable: true, required_action: '修复执行条件后重新运行 aria:retry' };
  }

  return { retryable: false, required_action: '人工处理并补齐合法工件' };
}
