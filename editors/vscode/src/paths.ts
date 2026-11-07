// Helpers without a `vscode` dependency (unit-tested in test/).
import * as path from 'path';

/**
 * Translates the folder that was right-clicked into the CLI's `--path` (a folder directly
 * under `src/`). Returns `null` when the folder cannot be expressed that way.
 */
export function featureDirFor(project: { src: string }, folder: string | undefined): string | undefined | null {
  if (!folder) return undefined;
  const rel = path.relative(project.src, folder);
  if (rel === '' || rel === '.') return undefined;
  if (rel.startsWith('..') || path.isAbsolute(rel)) return undefined;
  const parts = rel.split(/[\\/]/).filter(Boolean);
  if (parts.length === 1) return parts[0];
  return null;
}

export interface FileChange {
  kind: 'CREATE' | 'UPDATE' | 'SKIP';
  file: string;
}

export function parseChanges(output: string, cwd: string): FileChange[] {
  const changes: FileChange[] = [];
  for (const line of output.split(/\r?\n/)) {
    const m = /^(CREATE|UPDATE|SKIP)\s+(.+?)(?:\s+\((?:\d+ bytes|could not update automatically)\))?\s*$/.exec(line.trim());
    if (m) changes.push({ kind: m[1] as FileChange['kind'], file: path.resolve(cwd, m[2]) });
  }
  return changes;
}

