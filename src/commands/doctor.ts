import { buildCapabilityReport } from '../diagnostics/capability-report.js';

export async function doctorCommand(): Promise<string> {
  return buildCapabilityReport();
}
