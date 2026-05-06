import { spawnSync } from 'node:child_process';

const databaseUrl = process.env.DATABASE_URL ?? 'postgres://sanchoris:sanchoris@127.0.0.1:54329/sanchoris';
const schemaPath = process.argv[2] ?? 'db/schema.sql';

const command = process.env.PSQLDEF ? 'psqldef' : 'psql';
const args = command === 'psqldef'
  ? ['--file', schemaPath, databaseUrl]
  : [databaseUrl, '--file', schemaPath, '--set', 'ON_ERROR_STOP=1'];

const result = spawnSync(command, args, { stdio: 'inherit' });

if (result.error) {
  console.error(`${command} failed to start: ${result.error.message}`);
  process.exit(1);
}

process.exit(result.status ?? 1);
