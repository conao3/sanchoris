import { createHash } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const envPath = '.env';
const beginMarker = '# BEGIN WORKTRUNK ENV';
const endMarker = '# END WORKTRUNK ENV';

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

function sanitizeName(value) {
  const normalized = value
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, '_')
    .replace(/^_+|_+$/g, '');
  const hash = createHash('sha1').update(value).digest('hex').slice(0, 6);
  const base = normalized || 'branch';
  return `${base}_${hash}`.slice(0, 48).replace(/_+$/g, '');
}

function dynamoPort(value) {
  const hash = createHash('sha1').update(value).digest();
  return 8100 + (hash.readUInt16BE(0) % 800);
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
const name = sanitizeName(branch);
const port = dynamoPort(`sanchoris:${branch}`);

const block = `${beginMarker}
SANCHORIS_DYNAMO_PORT=${port}
AWS_ENDPOINT_URL_DYNAMODB=http://127.0.0.1:${port}
SANCHORIS_TABLE_MAIN=sanchoris-${name}-main
SANCHORIS_TABLE_AUDIT=sanchoris-${name}-audit
${endMarker}`;

const current = existsSync(envPath) ? readFileSync(envPath, 'utf8') : '';
const next = upsertManagedBlock(current, block);
writeFileSync(envPath, next);

console.log(`Updated ${envPath} Worktrunk block for ${branch}: DynamoDB Local on port ${port}`);
