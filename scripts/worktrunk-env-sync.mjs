import { createHash } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const envPath = '.env';
const beginMarker = '# BEGIN WORKTRUNK ENV';
const endMarker = '# END WORKTRUNK ENV';
const postgresPort = '54329';

function readStdinJson() {
  try {
    const input = readFileSync(0, 'utf8').trim();
    if (!input) {
      return null;
    }
    return JSON.parse(input);
  } catch {
    return null;
  }
}

function currentBranch() {
  const result = spawnSync('git', ['branch', '--show-current'], { encoding: 'utf8' });
  if (result.status !== 0) {
    throw new Error(result.stderr || 'Failed to read current git branch.');
  }
  const branch = result.stdout.trim();
  if (!branch) {
    throw new Error('Current git branch is empty.');
  }
  return branch;
}

function sanitizeDb(value) {
  const normalized = value
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .replace(/^[0-9]/, 'b_$&');
  const hash = createHash('sha1').update(value).digest('hex').slice(0, 6);
  const base = normalized || 'branch';
  return `${base}_${hash}`.slice(0, 63).replace(/_+$/g, '');
}

function bindHost(value) {
  const hash = createHash('sha1').update(value).digest();
  const octets = [hash[0], hash[1], hash[2]].map((byte) => (byte % 254) + 1);
  return `127.${octets[0]}.${octets[1]}.${octets[2]}`;
}

function upsertManagedBlock(current, block) {
  const pattern = new RegExp(`${beginMarker.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}[\\s\\S]*?${endMarker.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`, 'm');
  if (pattern.test(current)) {
    return current.replace(pattern, block);
  }
  const trimmed = current.trimEnd();
  return `${trimmed}${trimmed ? '\n\n' : ''}${block}\n`;
}

const context = readStdinJson();
const branch = context?.branch || currentBranch();
const composeProjectName = `sanchoris_${sanitizeDb(branch)}`;
const host = bindHost(`sanchoris:${branch}`);
const databaseUrl = `postgres://sanchoris:sanchoris@${host}:${postgresPort}/sanchoris`;

const block = `${beginMarker}
COMPOSE_PROJECT_NAME=${composeProjectName}
BIND_HOST=${host}
DATABASE_URL=${databaseUrl}
${endMarker}`;

const current = existsSync(envPath) ? readFileSync(envPath, 'utf8') : '';
const next = upsertManagedBlock(current, block);
writeFileSync(envPath, next);

console.log(`Updated ${envPath} Worktrunk block for ${branch}: ${host}:${postgresPort}`);
