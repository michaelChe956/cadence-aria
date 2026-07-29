import type { ValidatorFindingDto } from "../../api/types";

interface DraftValidationFailureNoticeProps {
  findings: ValidatorFindingDto[] | null | undefined;
}

export function DraftValidationFailureNotice({ findings }: DraftValidationFailureNoticeProps) {
  const availableFindings = findings ?? [];
  const summaryFindings = availableFindings.slice(0, 3);

  return (
    <section
      role="alert"
      aria-live="assertive"
      className="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-800"
    >
      <strong>Draft 校验失败，暂不能接受</strong>
      {availableFindings.length > 0 ? <span>（{availableFindings.length} 项）</span> : null}
      {availableFindings.length === 0 ? (
        <p className="mt-2">
          校验详情暂不可用，请根据 Draft 内容重写；可点击根据校验错误重写后再次生成。
        </p>
      ) : (
        <>
          <ul className="mt-2 list-disc space-y-1 pl-5">
            {summaryFindings.map((finding) => (
              <li key={finding.finding_id}>
                <code>{finding.code ?? finding.finding_id}</code>：{finding.message}
              </li>
            ))}
          </ul>
          {availableFindings.length > 0 ? (
            <details className="mt-2">
              <summary>查看全部 {availableFindings.length} 项错误</summary>
              <ul className="mt-2 list-disc space-y-1 pl-5">
                {availableFindings.map((finding) => (
                  <li key={finding.finding_id}>
                    <code>{finding.code ?? finding.finding_id}</code>：{finding.message}
                  </li>
                ))}
              </ul>
            </details>
          ) : null}
        </>
      )}
    </section>
  );
}
