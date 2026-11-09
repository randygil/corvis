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

export interface CorvisPathResult {
  /** The folder to pass as `CORVIS_PATH` (the `crates/corvis` crate folder). */
  path?: string;
  /** Why the configured value cannot be used. */
  error?: string;
}

/**
 * Resolves the `corvis.new.corvisPath` setting. Empty gives `{}`; a valid value gives the
 * `crates/corvis` folder (a repository root is accepted and turned into it); anything else
 * gives an error message. `home` and `exists` are parameters so no file system is needed.
 */
export function resolveCorvisPath(value: string | undefined, home: string, exists: (p: string) => boolean): CorvisPathResult {
  const trimmed = (value ?? '').trim();
  if (!trimmed) return {};
  const expanded = trimmed.startsWith('~') ? path.join(home, trimmed.slice(1)) : trimmed;
  if (!path.isAbsolute(expanded)) {
    return { error: `corvis.new.corvisPath must be an absolute path (got "${trimmed}")` };
  }
  const resolved = exists(path.join(expanded, 'crates', 'corvis-cli', 'Cargo.toml')) ? path.join(expanded, 'crates', 'corvis') : expanded;
  if (!exists(path.join(resolved, 'Cargo.toml'))) {
    return { error: `corvis.new.corvisPath points at ${resolved}, which has no Cargo.toml` };
  }
  return { path: resolved };
}

