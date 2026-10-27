import { spawnSync } from 'node:child_process';

const databaseUrl = process.env.DATABASE_URL;
if (!databaseUrl) {
  console.error('DATABASE_URL is not set');
  process.exit(1);
}

const args = process.argv.slice(2);
const reset = args.includes('--reset');
const positional = args.filter((arg) => arg !== '--reset');
if (positional.length > 1) {
  console.error('usage: db-apply.mjs [--reset] [schema-path]');
  process.exit(1);
}
const schemaPath = positional[0] ?? 'db/schema.sql';

function psql(psqlArgs) {
  const result = spawnSync('psql', [databaseUrl, '--set', 'ON_ERROR_STOP=1', ...psqlArgs], { stdio: 'inherit' });
  if (result.error) {
    console.error(`psql failed to start: ${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

if (reset) {
  psql(['--command', 'DROP SCHEMA IF EXISTS sanchoris CASCADE']);
}
psql(['--file', schemaPath]);
