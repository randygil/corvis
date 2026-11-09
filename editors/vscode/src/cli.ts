import * as cp from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';

import { CargoProject, expandHome, findFrameworkRoot } from './project';

/** How to launch the `corvis` CLI: `command args... <corvis arguments>`. */
export interface CliInvocation {
  command: string;
  args: string[];
  /** Human readable form for logs. */
  label: string;
  source: 'setting' | 'path' | 'cargo-bin' | 'cargo-run';
}

export interface RunResult {
  code: number | null;
  output: string;
}

const ANSI = /\x1b\[[0-9;]*[A-Za-z]/g;

export function stripAnsi(s: string): string {
  return s.replace(ANSI, '');
}

let output: vscode.OutputChannel | undefined;

export function outputChannel(): vscode.OutputChannel {
  if (!output) output = vscode.window.createOutputChannel('Corvis');
  return output;
}

let pathProbe: Promise<boolean> | undefined;

export function resetCliCache(): void {
  pathProbe = undefined;
}

function probe(command: string): Promise<boolean> {
  return new Promise((resolve) => {
    try {
      const child = cp.spawn(command, ['--version'], { shell: needsShell(command), windowsHide: true });
      const timer = setTimeout(() => {
        child.kill();
        resolve(false);
      }, 5000);
      child.on('error', () => {
        clearTimeout(timer);
        resolve(false);
      });
      child.on('exit', (code) => {
        clearTimeout(timer);
        resolve(code === 0);
      });
    } catch {
      resolve(false);
    }
  });
}

function needsShell(command: string): boolean {
  return process.platform === 'win32' && /\.(cmd|bat)$/i.test(command);
}

function cargoBinCorvis(): string | undefined {
  const home = process.env.CARGO_HOME || path.join(os.homedir(), '.cargo');
  const exe = path.join(home, 'bin', process.platform === 'win32' ? 'corvis.exe' : 'corvis');
  return fs.existsSync(exe) ? exe : undefined;
}

/**
 * Finds the CLI: the `corvis.cli.path` setting, `corvis` on PATH, `~/.cargo/bin/corvis`,
 * or `cargo run` against the Corvis repository.
 */
export async function resolveCli(project?: CargoProject): Promise<CliInvocation | undefined> {
  const configured = vscode.workspace.getConfiguration('corvis').get<string>('cli.path')?.trim();
  if (configured) {
    const command = expandHome(configured);
    return { command, args: [], label: command, source: 'setting' };
  }
  pathProbe ??= probe('corvis');
  if (await pathProbe) return { command: 'corvis', args: [], label: 'corvis', source: 'path' };
  const bin = cargoBinCorvis();
  if (bin) return { command: bin, args: [], label: bin, source: 'cargo-bin' };
  const framework = findFrameworkRoot(project);
  if (framework) {
    const manifest = path.join(framework, 'crates', 'corvis-cli', 'Cargo.toml');
    return {
      command: 'cargo',
      args: ['run', '-q', '--manifest-path', manifest, '--'],
      label: `cargo run -q --manifest-path ${manifest} --`,
      source: 'cargo-run',
    };
  }
  return undefined;
}

/** Like `resolveCli`, but explains what to do when nothing is found. */
export async function requireCli(project?: CargoProject): Promise<CliInvocation | undefined> {
  const cli = await resolveCli(project);
  if (cli) return cli;
  const choice = await vscode.window.showErrorMessage(
    'Corvis: the `corvis` CLI was not found. Install it with `cargo install --path crates/corvis-cli` from the Corvis repository, or tell the extension where the repository is.',
    'Select Corvis repository…',
    'Open Settings',
  );
  if (choice === 'Open Settings') {
    await vscode.commands.executeCommand('workbench.action.openSettings', 'corvis.cli.path');
  } else if (choice === 'Select Corvis repository…') {
    const picked = await vscode.window.showOpenDialog({ canSelectFolders: true, canSelectFiles: false, openLabel: 'Use this Corvis repository' });
    if (picked?.[0]) {
      await vscode.workspace.getConfiguration('corvis').update('frameworkPath', picked[0].fsPath, vscode.ConfigurationTarget.Global);
      return resolveCli(project);
    }
  }
  return undefined;
}

/** Runs the CLI and streams its output to the "Corvis" output channel. */
export function runCli(cli: CliInvocation, args: string[], cwd: string, token?: vscode.CancellationToken, extraEnv?: Record<string, string>): Promise<RunResult> {
  const out = outputChannel();
  const shown = [cli.label, ...args.map((a) => (/\s/.test(a) ? JSON.stringify(a) : a))].join(' ');
  out.appendLine(`> ${shown}`);
  out.appendLine(`  (cwd: ${cwd})`);
  if (extraEnv?.CORVIS_PATH) out.appendLine(`  (CORVIS_PATH: ${extraEnv.CORVIS_PATH})`);
  return new Promise((resolve) => {
    let buffer = '';
    let child: cp.ChildProcess;
    try {
      child = cp.spawn(cli.command, [...cli.args, ...args], {
        cwd,
        shell: needsShell(cli.command),
        windowsHide: true,
        env: { ...process.env, ...extraEnv, NO_COLOR: '1', CARGO_TERM_COLOR: 'never' },
      });
    } catch (e) {
      out.appendLine(String(e));
      resolve({ code: -1, output: String(e) });
      return;
    }
    const onData = (d: Buffer) => {
      const text = stripAnsi(d.toString('utf8'));
      buffer += text;
      out.append(text);
    };
    child.stdout?.on('data', onData);
    child.stderr?.on('data', onData);
    token?.onCancellationRequested(() => child.kill());
    child.on('error', (e) => {
      out.appendLine(`\n${e.message}`);
      resolve({ code: -1, output: buffer + e.message });
    });
    child.on('close', (code) => {
      out.appendLine(code === 0 ? '' : `\n(exit code ${code})`);
      resolve({ code, output: buffer });
    });
  });
}

/** Runs a long-lived CLI command (`start`, `build`) as a VS Code task in the terminal. */
export async function runCliTask(cli: CliInvocation, args: string[], cwd: string, name: string): Promise<void> {
  const exec = new vscode.ProcessExecution(cli.command, [...cli.args, ...args], { cwd });
  const task = new vscode.Task({ type: 'corvis', command: args.join(' ') }, vscode.TaskScope.Workspace, name, 'corvis', exec, []);
  task.presentationOptions = { reveal: vscode.TaskRevealKind.Always, panel: vscode.TaskPanelKind.Dedicated, clear: true };
  await vscode.tasks.executeTask(task);
}
