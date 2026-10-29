import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { parseArgs } from 'node:util';
import { aws, e2eTenantSlug, environment, exportsOf, fail, run, sleep } from './lib/aws-env.mjs';
import { taskName, taskTable } from './lib/e2e-task.mjs';

const usage = 'usage: node scripts/e2e-aws.mjs <dev|prd> --arch arm64|x86_64 [--title <issue title>] [--timeout-minutes <n>]';

const labels = { arm64: 'sanchoris', x86_64: 'sanchoris-x86' };
const endStatuses = ['SUCCEEDED', 'FAILED', 'ABORTED', 'NEEDS_YOU'];
const logDataLength = 400;

let parsed;
try {
  parsed = parseArgs({
    allowPositionals: true,
    options: {
      arch: { type: 'string' },
      title: { type: 'string' },
      'timeout-minutes': { type: 'string' },
    },
  });
} catch (error) {
  fail(`${error.message}\n${usage}`);
}
const { values, positionals } = parsed;
if (positionals.length !== 1 || !labels[values.arch]) {
  fail(usage);
}
const env = environment(positionals[0]);
const arch = values.arch;
const label = labels[arch];
const timeoutMs = Number(values['timeout-minutes'] ?? 45) * 60_000;
if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
  fail(`--timeout-minutes must be a positive number\n${usage}`);
}
const repo = env.e2eRepo;
const startedAt = Date.now();
const timings = {};
const mark = (name) => {
  timings[name] = (Date.now() - startedAt) / 1000;
};

const config = Buffer.from(
  JSON.parse(run('gh', ['api', `repos/${repo}/contents/sanchoris.yaml`])).content,
  'base64',
).toString('utf8');
if (!config.includes(`label: [${label}]`)) {
  fail(`${repo} sanchoris.yaml has no trigger on label ${label} (for ${arch})`);
}

const exports = exportsOf(env);
const table = exports.get('SanchorisMainTable');
const cluster = exports.get('SanchorisCluster');

const tasks = taskTable((args) => aws(env, ['dynamodb', ...args]), table);
const tenant = tasks.tenantIdBySlug(e2eTenantSlug);

const title = values.title ?? `Add a function triple${startedAt.toString(36)}(x) that returns 3 * x, with a test`;
const issueUrl = run('gh', ['issue', 'create', '-R', repo, '--title', title, '--body', `sanchoris e2e (${arch}).`]).trim();
const issue = Number(issueUrl.split('/').pop());
mark('issueCreated');
run('gh', ['issue', 'edit', String(issue), '-R', repo, '--add-label', label]);
mark('labeled');
process.stderr.write(`e2e ${env.name} ${arch}: ${issueUrl} labeled ${label}\n`);

const repositoryId = run('gh', ['api', `repos/${repo}`, '--jq', '.id']).trim();
const name = taskName(repositoryId, issue);

async function waitFor(check, describe) {
  let delay = 3000;
  let last;
  while (Date.now() - startedAt < timeoutMs) {
    const value = check();
    const state = describe(value);
    if (state !== last) {
      process.stderr.write(`e2e ${env.name} ${arch}: ${state} at ${((Date.now() - startedAt) / 1000).toFixed(0)}s\n`);
      last = state;
    }
    if (value?.done) {
      return value;
    }
    await sleep(Math.min(delay, Math.max(0, timeoutMs - (Date.now() - startedAt))));
    delay = Math.min(Math.round(delay * 1.5), 30_000);
  }
  return null;
}

let outcome = 'timeout';
let task = null;
const found = await waitFor(
  () => {
    const t = tasks.findTask(tenant, startedAt, name);
    return t ? { done: true, task: t } : null;
  },
  (value) => (value ? 'task created' : 'waiting for the task'),
);
if (found) {
  mark('taskCreated');
  const ended = await waitFor(
    () => {
      const t = tasks.readTask(tenant, found.task.id);
      return { done: endStatuses.includes(t.status), task: t };
    },
    (value) => `task ${value.task.status}`,
  );
  task = ended?.task ?? tasks.readTask(tenant, found.task.id);
  if (ended) {
    mark('taskEnded');
    outcome = task.status;
  }
}

function collect() {
  const { runs, workspaces, outputs } = tasks.taskItems(tenant, task.id);

  const runDetails = runs.map((r) => {
    const logs = tasks.runLogs(tenant, r.id);
    const relay = aws(env, [
      'logs',
      'filter-log-events',
      '--log-group-name',
      `/aws/ecs/${env.prefix}-sanchoris-relay`,
      '--start-time',
      String(startedAt - 60_000),
      '--filter-pattern',
      `{ $.run = "${r.id}" }`,
    ]).events.map((event) => JSON.parse(event.message));
    return {
      id: r.id,
      number: r.number,
      status: r.status,
      session: r.session_name,
      turns: r.turns,
      costUsd: r.cost_usd,
      error: r.error,
      summary: r.summary,
      startedAt: r.started_at,
      finishedAt: r.finished_at,
      logs,
      relay,
    };
  });

  const taskArns = workspaces.map((w) => w.platform_ref).filter(Boolean);
  const ecs = taskArns.length === 0
    ? []
    : aws(env, ['ecs', 'describe-tasks', '--cluster', cluster, '--tasks', ...taskArns]).tasks.map((t) => ({
        taskArn: t.taskArn,
        lastStatus: t.lastStatus,
        cpu: t.cpu,
        memory: t.memory,
        capacityProviderName: t.capacityProviderName,
        architecture: t.attributes?.find((a) => a.name === 'ecs.cpu-architecture')?.value,
        stopCode: t.stopCode,
        stoppedReason: t.stoppedReason,
        exitCodes: t.containers.map((c) => ({ name: c.name, exitCode: c.exitCode, reason: c.reason })),
        startedAt: t.startedAt,
        stoppedAt: t.stoppedAt,
      }));

  const pullRequests = outputs
    .filter((o) => o.output_kind === 'pull_request')
    .map((o) => {
      const pr = JSON.parse(run('gh', ['pr', 'view', o.url, '--json', 'state,statusCheckRollup']));
      return {
        url: o.url,
        state: pr.state,
        checks: pr.statusCheckRollup.map((c) => ({ name: c.name ?? c.context, status: c.status, conclusion: c.conclusion ?? c.state })),
      };
    });

  return {
    runs: runDetails,
    workspaces: workspaces.map((w) => ({
      id: w.id,
      run: w.run_id,
      architecture: w.architecture,
      cpu: w.cpu,
      memoryMib: w.memory_mib,
      capacityProvider: w.capacity_provider,
      platformRef: w.platform_ref,
      startedAt: w.started_at,
      signaledAt: w.signaled_at,
      stoppedAt: w.stopped_at,
    })),
    ecs,
    outputs: outputs.map((o) => ({ kind: o.output_kind, url: o.url })),
    pullRequests,
  };
}

const details = task ? collect() : null;
const ok =
  outcome === 'SUCCEEDED' &&
  details.workspaces.length > 0 &&
  details.workspaces.every((w) => w.architecture === arch) &&
  details.pullRequests.every((pr) => pr.checks.every((c) => c.conclusion === 'SUCCESS'));

const report = {
  env: env.name,
  arch,
  ok,
  outcome,
  issue: issueUrl,
  label,
  timings,
  task: task && {
    id: task.id,
    status: task.status,
    error: task.error,
    cause: task.cause,
    createdAt: task.created_at,
    startedAt: task.started_at,
    finishedAt: task.finished_at,
  },
  ...details,
};

const dir = join(run('git', ['rev-parse', '--show-toplevel']).trim(), '.sanchoris', 'e2e-aws');
mkdirSync(dir, { recursive: true });
const file = join(dir, `${env.name}-${arch}-${issue}.json`);
writeFileSync(file, `${JSON.stringify(report, null, 2)}\n`);

const truncate = (value) => (typeof value === 'string' && value.length > logDataLength ? `${value.slice(0, logDataLength)}...` : value);
const brief = {
  ...report,
  runs: report.runs?.map((r) => ({
    ...r,
    logs: r.logs.map((entry) => ({ ...entry, data: truncate(entry.data) })),
    relay: Object.values(
      r.relay.reduce((acc, entry) => {
        const key = `${entry.host}:${entry.port} ${entry.allowed ? 'allowed' : 'denied'}`;
        acc[key] ??= { host: entry.host, port: entry.port, allowed: entry.allowed, rule: entry.rule, reason: entry.reason, connects: 0 };
        acc[key].connects += 1;
        return acc;
      }, {}),
    ),
  })),
  report: file,
};
console.log(JSON.stringify(brief, null, 2));
process.exit(ok ? 0 : 1);
