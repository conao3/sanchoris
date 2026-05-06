import { mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const tempDir = mkdtempSync(join(tmpdir(), 'sanchoris-postgres-'));
const dataDir = join(tempDir, 'data');
const socketDir = join(tempDir, 'socket');
const dbName = 'sanchoris_schema_check';
mkdirSync(socketDir, { recursive: true });
const env = { ...process.env, PGHOST: socketDir, PGDATABASE: dbName };

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    stdio: options.capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
    encoding: 'utf8',
    env: options.env ?? env,
  });

  if (result.error) {
    throw new Error(`${command} failed to start: ${result.error.message}`);
  }

  if (result.status !== 0) {
    const stderr = result.stderr ? `\n${result.stderr}` : '';
    throw new Error(`${command} ${args.join(' ')} failed with exit code ${result.status}${stderr}`);
  }

  return result.stdout ?? '';
}

function stopPostgres() {
  spawnSync('pg_ctl', ['-D', dataDir, '-m', 'fast', 'stop'], { stdio: 'ignore', env });
}

try {
  run('initdb', ['-D', dataDir, '--encoding=UTF8', '--locale=C']);
  run('pg_ctl', ['-D', dataDir, '-o', `-k ${socketDir}`, '-w', 'start']);
  run('createdb', [dbName]);
  run('psql', ['--file', 'db/schema.sql', '--set', 'ON_ERROR_STOP=1']);

  const tableCount = run(
    'psql',
    [
      '--tuples-only',
      '--no-align',
      '--command',
      "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'sanchoris';",
    ],
    { capture: true },
  ).trim();

  const requiredTables = [
    'project_profiles',
    'conversations',
    'chat_messages',
    'tasks',
    'workflow_specs',
    'workflow_blocks',
    'workflow_edges',
    'workflow_runs',
    'workflow_step_runs',
    'workspaces',
    'verification_results',
    'reviews',
    'pull_requests',
    'merges',
    'events',
    'users',
  ];

  const missingTables = requiredTables.filter((table) => {
    const output = run(
      'psql',
      [
        '--tuples-only',
        '--no-align',
        '--command',
        `SELECT to_regclass('sanchoris.${table}') IS NOT NULL;`,
      ],
      { capture: true },
    ).trim();
    return output !== 't';
  });

  if (missingTables.length > 0) {
    throw new Error(`Missing required tables: ${missingTables.join(', ')}`);
  }

  console.log(`sanchoris schema check passed: ${tableCount} tables in schema sanchoris`);
} finally {
  stopPostgres();
  rmSync(tempDir, { recursive: true, force: true });
}
