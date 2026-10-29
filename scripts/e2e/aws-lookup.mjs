import { spawnSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import { taskName, taskTable } from '../lib/e2e-task.mjs';

const { values } = parseArgs({
  options: {
    endpoint: { type: 'string' },
    table: { type: 'string' },
    slug: { type: 'string' },
    'repository-id': { type: 'string' },
    issue: { type: 'string' },
    'since-ms': { type: 'string' },
  },
});
const missing = ['endpoint', 'table', 'slug', 'repository-id', 'issue', 'since-ms'].filter((key) => !values[key]);
if (missing.length > 0) {
  console.error(`aws-lookup: missing ${missing.map((key) => `--${key}`).join(' ')}`);
  process.exit(2);
}

function dynamodb(args) {
  const result = spawnSync('aws', ['dynamodb', ...args, '--endpoint-url', values.endpoint, '--output', 'json'], {
    encoding: 'utf8',
    env: {
      PATH: process.env.PATH,
      HOME: process.env.HOME,
      AWS_ACCESS_KEY_ID: 'test',
      AWS_SECRET_ACCESS_KEY: 'test',
      AWS_REGION: 'us-east-1',
      AWS_CONFIG_FILE: '/dev/null',
      AWS_SHARED_CREDENTIALS_FILE: '/dev/null',
    },
  });
  if (result.error) {
    throw new Error(`aws failed to start: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`aws dynamodb ${args.join(' ')} exited with ${result.status}\n${result.stderr.trim()}`);
  }
  return result.stdout.trim() === '' ? null : JSON.parse(result.stdout);
}

const tasks = taskTable(dynamodb, values.table);
const tenant = tasks.tenantIdBySlug(values.slug);
const found = tasks.findTask(tenant, Number(values['since-ms']), taskName(values['repository-id'], values.issue));
if (!found) {
  console.error(`aws-lookup: no task ${taskName(values['repository-id'], values.issue)} in tenant ${tenant}`);
  process.exit(1);
}
const task = tasks.readTask(tenant, found.id);
const { runs, workspaces, outputs } = tasks.taskItems(tenant, task.id);
console.log(
  JSON.stringify({
    tenant,
    task: { id: task.id, status: task.status },
    runs: runs.map((r) => ({ id: r.id, status: r.status, logs: tasks.runLogs(tenant, r.id).length })),
    workspaces: workspaces.map((w) => ({ id: w.id, run: w.run_id })),
    outputs: outputs.map((o) => ({ kind: o.output_kind, url: o.url })),
  }),
);
