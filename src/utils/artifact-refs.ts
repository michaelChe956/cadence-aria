import path from 'node:path';

export function toConsumedSpecRef(specRef: string): string {
  return path.posix.join('artifacts', path.posix.basename(specRef));
}

export function toArtifactRef(inputPath: string): string {
  return path.posix.join('artifacts', path.posix.basename(inputPath));
}
