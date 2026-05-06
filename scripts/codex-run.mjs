#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import fs from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

const defaultRunsDir = '.sanchoris/codex-runs';

function usage() {
  console.log(`Usage:
  node scripts/codex-run.mjs --prompt <file> [options] -- <command> [args...]

Options:
  --prompt <file>       Prompt file to record for the run.
  --runs-dir <dir>      Directory where run folders are created. Defaults to ${defaultRunsDir}.
  --name <name>         Human-readable run name used in the run directory.
  --verify <command>    Verification command to run after the task command. Repeatable.
  --no-copy-prompt      Store only the prompt file path instead of copying the prompt.
  -h, --help            Show this help.

Example:
  node scripts/codex-run.mjs --prompt tasks/example.md --verify "pnpm check" -- \\
    codex exec --ask-for-approval never < tasks/example.md
`);
}

function parseArgs(argv) {
  const options = {
    copyPrompt: true,
    name: 'codex-run',
    prompt: undefined,
    runsDir: defaultRunsDir,
    verify: [],
  };

  let index = 0;
  for (; index < argv.length; index += 1) {
    const arg = argv[index];

    if (arg === '--') {
      index += 1;
      break;
    }

    if (arg === '-h' || arg === '--help') {
      options.help = true;
      return { command: [], options };
    }

    if (arg === '--no-copy-prompt') {
      options.copyPrompt = false;
      continue;
    }

    if (arg === '--prompt' || arg === '--runs-dir' || arg === '--name' || arg === '--verify') {
      const value = argv[index + 1];
      if (!value) {
        throw new Error(`${arg} requires a value.`);
      }

      index += 1;
      if (arg === '--verify') {
        options.verify.push(value);
      } else if (arg === '--runs-dir') {
        options.runsDir = value;
      } else {
        options[arg.slice(2)] = value;
      }
      continue;
    }

    throw new Error(`Unknown option: ${arg}`);
  }

  return { command: argv.slice(index), options };
}

function safeName(value) {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 80) || 'codex-run';
}

function nowIsoForPath() {
  return new Date().toISOString().replaceAll(':', '').replace(/\.\d{3}Z$/, 'Z');
}

function writeJson(file, value) {
  return fs.writeFile(`${file}.tmp`, `${JSON.stringify(value, null, 2)}\n`).then(() =>
    fs.rename(`${file}.tmp`, file),
  );
}

function openLogs(runDir) {
  return {
    combined: createWriteStream(path.join(runDir, 'combined.log'), { flags: 'a' }),
    stderr: createWriteStream(path.join(runDir, 'stderr.log'), { flags: 'a' }),
    stdout: createWriteStream(path.join(runDir, 'stdout.log'), { flags: 'a' }),
  };
}

function closeLogs(logs) {
  return Promise.all(
    Object.values(logs).map(
      (stream) =>
        new Promise((resolve, reject) => {
          stream.end((error) => {
            if (error) {
              reject(error);
            } else {
              resolve();
            }
          });
        }),
    ),
  );
}

function runChild(command, args, cwd, logs) {
  return new Promise((resolve) => {
    const child = spawn(command, args, {
      cwd,
      env: process.env,
      shell: false,
      stdio: ['inherit', 'pipe', 'pipe'],
    });

    child.stdout.on('data', (chunk) => {
      logs.stdout.write(chunk);
      logs.combined.write(chunk);
      process.stdout.write(chunk);
    });

    child.stderr.on('data', (chunk) => {
      logs.stderr.write(chunk);
      logs.combined.write(chunk);
      process.stderr.write(chunk);
    });

    child.on('error', (error) => {
      logs.stderr.write(`${error.stack ?? error.message}\n`);
      logs.combined.write(`${error.stack ?? error.message}\n`);
      resolve({ error: error.message, exitCode: 1, pid: child.pid ?? null, signal: null });
    });

    child.on('exit', (exitCode, signal) => {
      resolve({ exitCode, pid: child.pid ?? null, signal });
    });
  });
}

function runVerification(command, cwd, runDir, index) {
  const filePrefix = `verify-${String(index + 1).padStart(2, '0')}`;
  const stdoutPath = path.join(runDir, `${filePrefix}-stdout.log`);
  const stderrPath = path.join(runDir, `${filePrefix}-stderr.log`);
  const stdout = createWriteStream(stdoutPath, { flags: 'a' });
  const stderr = createWriteStream(stderrPath, { flags: 'a' });
  const startedAt = new Date().toISOString();

  return new Promise((resolve) => {
    const child = spawn(command, {
      cwd,
      env: process.env,
      shell: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });

    child.stdout.on('data', (chunk) => {
      stdout.write(chunk);
      process.stdout.write(chunk);
    });

    child.stderr.on('data', (chunk) => {
      stderr.write(chunk);
      process.stderr.write(chunk);
    });

    child.on('error', (error) => {
      stderr.write(`${error.stack ?? error.message}\n`);
      resolve({
        command,
        error: error.message,
        exit_code: 1,
        finished_at: new Date().toISOString(),
        log_paths: { stderr: stderrPath, stdout: stdoutPath },
        pid: child.pid ?? null,
        signal: null,
        started_at: startedAt,
        status: 'failed',
      });
    });

    child.on('exit', (exitCode, signal) => {
      stdout.end();
      stderr.end();
      resolve({
        command,
        exit_code: exitCode,
        finished_at: new Date().toISOString(),
        log_paths: { stderr: stderrPath, stdout: stdoutPath },
        pid: child.pid ?? null,
        signal,
        started_at: startedAt,
        status: exitCode === 0 ? 'passed' : 'failed',
      });
    });
  });
}

async function main() {
  const { command, options } = parseArgs(process.argv.slice(2));
  if (options.help) {
    usage();
    return 0;
  }

  if (!options.prompt) {
    throw new Error('--prompt is required.');
  }

  if (command.length === 0) {
    throw new Error('A command after -- is required.');
  }

  const cwd = process.cwd();
  const promptPath = path.resolve(cwd, options.prompt);
  await fs.access(promptPath);

  const runId = `${nowIsoForPath()}-${safeName(options.name)}-${process.pid}`;
  const runDir = path.resolve(cwd, options.runsDir, runId);
  await fs.mkdir(runDir, { recursive: true });

  const copiedPromptPath = options.copyPrompt ? path.join(runDir, 'prompt.md') : null;
  if (copiedPromptPath) {
    await fs.copyFile(promptPath, copiedPromptPath);
  }

  const logs = openLogs(runDir);
  const metadataPath = path.join(runDir, 'metadata.json');
  const summaryPath = path.join(runDir, 'summary.json');
  const startedAt = new Date().toISOString();
  const summary = {
    command,
    finished_at: null,
    log_paths: {
      combined: path.join(runDir, 'combined.log'),
      stderr: path.join(runDir, 'stderr.log'),
      stdout: path.join(runDir, 'stdout.log'),
    },
    prompt: {
      copied_path: copiedPromptPath,
      source_path: promptPath,
    },
    run_dir: runDir,
    run_id: runId,
    started_at: startedAt,
    status: 'running',
    task_exit_code: null,
    task_signal: null,
    verification: options.verify.map((verifyCommand) => ({
      command: verifyCommand,
      status: 'pending',
    })),
  };

  await writeJson(metadataPath, {
    cwd,
    parent_pid: process.ppid,
    pid: process.pid,
    run_dir: runDir,
    run_id: runId,
    started_at: startedAt,
  });
  await writeJson(summaryPath, summary);

  console.error(`Run directory: ${runDir}`);
  const taskResult = await runChild(command[0], command.slice(1), cwd, logs);

  summary.task_exit_code = taskResult.exitCode;
  summary.task_pid = taskResult.pid;
  summary.task_signal = taskResult.signal;
  summary.status = taskResult.exitCode === 0 ? 'verifying' : 'failed';
  if (taskResult.error) {
    summary.error = taskResult.error;
  }
  await writeJson(summaryPath, summary);

  const verificationResults = [];
  if (taskResult.exitCode === 0) {
    for (let index = 0; index < options.verify.length; index += 1) {
      const result = await runVerification(options.verify[index], cwd, runDir, index);
      verificationResults.push(result);
      summary.verification = [
        ...verificationResults,
        ...options.verify.slice(index + 1).map((verifyCommand) => ({
          command: verifyCommand,
          status: 'pending',
        })),
      ];
      await writeJson(summaryPath, summary);
    }
  } else {
    summary.verification = options.verify.map((verifyCommand) => ({
      command: verifyCommand,
      status: 'skipped',
    }));
  }

  const failedVerification = verificationResults.find((result) => result.status !== 'passed');
  summary.finished_at = new Date().toISOString();
  summary.status = taskResult.exitCode === 0 && !failedVerification ? 'passed' : 'failed';
  await writeJson(summaryPath, summary);
  await closeLogs(logs);

  return summary.status === 'passed' ? 0 : 1;
}

main()
  .then((exitCode) => {
    process.exitCode = exitCode;
  })
  .catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
