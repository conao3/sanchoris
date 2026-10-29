type Obj = Record<string, any>;

const MINUTE = 60_000;
const now = Date.now();

function ago(minutes: number): string {
  return new Date(now - minutes * MINUTE).toISOString();
}

function ahead(minutes: number): string {
  return new Date(now + minutes * MINUTE).toISOString();
}

function nextMonday9(): string {
  const date = new Date(now);
  date.setDate(date.getDate() + ((8 - date.getDay()) % 7 || 7));
  date.setHours(9, 0, 0, 0);
  return date.toISOString();
}

function dayOfMonthBefore(day: number): string {
  const date = new Date(now);
  if (date.getDate() < day) {
    date.setMonth(date.getMonth() - 1);
  }
  date.setDate(day);
  date.setHours(0, 0, 0, 0);
  return date.toISOString();
}

function monthAfter(iso: string): string {
  const date = new Date(iso);
  date.setMonth(date.getMonth() + 1);
  return date.toISOString();
}

let sequence = 0;
export function nextId(prefix: string): string {
  sequence += 1;
  return `${prefix}-${sequence}`;
}

export const plans: Obj[] = [
  { id: 'plan-free', name: 'Free', priceUsd: 0, billingUnit: 'TENANT', taskLimit: 5, workspaceHourLimit: 10, limitPeriod: 'TOTAL', memberLimit: 1, concurrencyLimit: 1, inboundEventsPerMinute: 20, retentionDays: 30, auditRetentionDays: 30, requiresPayment: false, isAdmin: false },
  { id: 'plan-starter', name: 'Starter', priceUsd: 20, billingUnit: 'TENANT', taskLimit: 10, workspaceHourLimit: 50, limitPeriod: 'MONTH', memberLimit: 3, concurrencyLimit: 2, inboundEventsPerMinute: 60, retentionDays: 90, auditRetentionDays: 365, requiresPayment: true, isAdmin: false },
  { id: 'plan-pro', name: 'Pro', priceUsd: 100, billingUnit: 'TENANT', taskLimit: 50, workspaceHourLimit: 250, limitPeriod: 'MONTH', memberLimit: 10, concurrencyLimit: 5, inboundEventsPerMinute: 120, retentionDays: 90, auditRetentionDays: 365, requiresPayment: true, isAdmin: false },
  { id: 'plan-max', name: 'Max', priceUsd: 200, billingUnit: 'TENANT', taskLimit: 200, workspaceHourLimit: 500, limitPeriod: 'MONTH', memberLimit: 30, concurrencyLimit: 10, inboundEventsPerMinute: 300, retentionDays: 90, auditRetentionDays: 365, requiresPayment: true, isAdmin: false },
];

const planByName = (name: string) => plans.find((plan) => plan.name === name) as Obj;

export const triggerTemplates: Obj[] = [
  { id: 'tpl-labeled-issue', name: 'labeled-issue', description: 'Labeled issue → agent → pull requests with green checks' },
  { id: 'tpl-chat-all', name: 'chat-all', description: 'Web chat across all projects' },
  { id: 'tpl-weekly-deps', name: 'weekly-deps', description: 'Weekly dependency updates with green checks' },
];

const users: Record<string, Obj> = {
  mika: { id: 'user-mika', email: 'mika@acme.dev', name: 'Mika', githubLogin: 'mika-dev' },
  sota: { id: 'user-sota', email: 'sota@acme.dev', name: 'Sota', githubLogin: 'sota-dev' },
  ken: { id: 'user-ken', email: 'ken@acme.dev', name: 'Ken', githubLogin: 'ken-dev' },
  aya: { id: 'user-aya', email: 'aya@acme.dev', name: 'Aya', githubLogin: 'aya-dev' },
  rin: { id: 'user-rin', email: 'rin@acme.dev', name: 'Rin', githubLogin: 'rin-dev' },
  yu: { id: 'user-yu', email: 'yu@acme.dev', name: 'Yu', githubLogin: null },
  hana: { id: 'user-hana', email: 'hana@globex.dev', name: 'Hana', githubLogin: 'hana-gx' },
};

export const viewerUser = users.mika;

const validOk = { valid: true, errors: [] };

function periodUsage(tasks: number, workspaceHours: number, seats: number, running: number, period: 'MONTH' | 'TOTAL') {
  const start = dayOfMonthBefore(12);
  return {
    periodStart: period === 'MONTH' ? start : ago(60 * 24 * 20),
    periodEnd: period === 'MONTH' ? monthAfter(start) : null,
    tasksThisPeriod: tasks,
    workspaceSecondsThisPeriod: workspaceHours * 3600,
    seatsUsed: seats,
    runningRuns: running,
  };
}

function buildAcme(): Obj {
  const tenant: Obj = {
    id: 'tenant-acme',
    slug: 'acme',
    name: 'acme',
    plan: planByName('Pro'),
    state: 'ACTIVE',
    suspendedReason: null,
    createdAt: ago(60 * 24 * 90),
    githubOwner: { kind: 'ORGANIZATION', login: 'acme' },
    githubAppInstallUrl: 'https://github.com/apps/sanchoris/installations/new',
    activity: {
      runningTasks: 3,
      runsToday: 14,
      costTodayUsd: 18.4,
      pullRequestsOpenedToday: 6,
      runsLast24h: [1, 1, 0, 2, 1, 1, 3, 2, 4, 3, 5, 4, 6, 5, 7, 4, 6, 8, 5, 6, 7, 5, 8, 7],
    },
  };

  const members: Obj[] = [];
  function member(user: Obj, role: string, tasksStarted: number, lastActiveMinutes: number | null): Obj {
    const item: Obj = {
      id: `member-${user.name.toLowerCase()}`,
      user,
      tenant: () => tenant,
      role,
      joinedAt: ago(60 * 24 * 60),
      lastActiveAt: lastActiveMinutes == null ? null : ago(lastActiveMinutes),
      tasksStarted,
      credentials: () => credentials.filter((credential) => credential.owner === item),
    };
    members.push(item);
    return item;
  }
  const mika = member(users.mika, 'OWNER', 31, 0);
  const sota = member(users.sota, 'ADMIN', 12, 120);
  member(users.ken, 'MEMBER', 6, 60 * 26);
  const aya = member(users.aya, 'MEMBER', 9, 180);
  member(users.rin, 'MEMBER', 2, 60 * 24 * 3);
  member(users.yu, 'MEMBER', 0, null);

  const credentials: Obj[] = [
    { id: 'cred-mika-claude', name: 'mika-claude', provider: 'CLAUDE', kind: 'SUBSCRIPTION', subscriptionPlan: 'Max', account: 'mika@acme.dev', scope: 'PERSONAL', owner: mika, status: 'CONNECTED', rejectedAt: null, lastUsedAt: ago(0), usage: { takenAt: ago(1), fiveHour: { utilization: 0.18, resetsAt: ahead(140) }, sevenDay: { utilization: 0.42, resetsAt: nextMonday9() } } },
    { id: 'cred-acme-claude-team', name: 'acme-claude-team', provider: 'CLAUDE', kind: 'SUBSCRIPTION', subscriptionPlan: 'Team', account: 'acme shared', scope: 'ORGANIZATION', owner: null, status: 'CONNECTED', rejectedAt: null, lastUsedAt: ago(2), usage: { takenAt: ago(1), fiveHour: { utilization: 0.55, resetsAt: ahead(90) }, sevenDay: { utilization: 0.71, resetsAt: nextMonday9() } } },
    { id: 'cred-acme-claude-key', name: 'acme-claude-key', provider: 'CLAUDE', kind: 'API_KEY', subscriptionPlan: null, account: 'sk-ant-…9f2c', scope: 'ORGANIZATION', owner: null, status: 'CONNECTED', rejectedAt: null, lastUsedAt: ago(60 * 24 * 3), usage: null },
    { id: 'cred-mika-codex', name: 'mika-codex', provider: 'CODEX', kind: 'SUBSCRIPTION', subscriptionPlan: 'Pro', account: 'mika@acme.dev', scope: 'PERSONAL', owner: mika, status: 'RECONNECT', rejectedAt: ago(62), lastUsedAt: ago(62), usage: null },
    { id: 'cred-aya-claude', name: 'aya-claude', provider: 'CLAUDE', kind: 'SUBSCRIPTION', subscriptionPlan: 'Pro', account: 'aya@acme.dev', scope: 'PERSONAL', owner: aya, status: 'CONNECTED', rejectedAt: null, lastUsedAt: ago(200), usage: null, hidden: true },
  ];

  const invitations: Obj[] = [
    { id: 'inv-ren', email: 'ren@acme.dev', role: 'MEMBER', state: 'PENDING', invitedBy: mika, expiresAt: ahead(60 * 24 * 6), tenantName: 'acme' },
    { id: 'inv-yuki', email: 'yuki@acme.dev', role: 'ADMIN', state: 'PENDING', invitedBy: sota, expiresAt: ahead(60 * 24 * 2), tenantName: 'acme' },
  ];

  const integrations: Obj[] = [
    { id: 'int-github', kind: 'GITHUB_INSTALLATION', status: 'CONNECTED', account: 'acme', repositories: ['acme/shop-web', 'acme/shop-api', 'acme/sanchoris-config'], manageUrl: 'https://github.com/organizations/acme/settings/installations', lastSyncedAt: ago(2), error: null },
    { id: 'int-slack', kind: 'SLACK', status: 'NOT_CONNECTED', account: null, repositories: [], manageUrl: null, lastSyncedAt: null, error: null },
  ];

  const changeRequests: Obj[] = [];
  const projects: Obj[] = [];
  const sessions: Obj[] = [];

  const configMain: Obj = {
    id: 'config-main',
    repository: 'acme/sanchoris-config',
    repositoryId: '812004417',
    status: 'ACTIVE',
    root: '/',
    branch: 'main',
    url: 'https://github.com/acme/sanchoris-config',
    newSkillUrl: 'https://github.com/acme/sanchoris-config/new/main/skills',
    revision: { id: 'rev-41', number: 41, commitSha: '9f2c4e1', committedAt: ago(60 * 5), validation: validOk },
    revisions: () => [{ id: 'rev-41', number: 41, commitSha: '9f2c4e1', committedAt: ago(60 * 5), validation: validOk }],
    syncedAt: ago(2),
    syncError: null,
    triggerCount: 3,
    skillCount: 3,
    projects: () => projects,
    changeRequests: (args: Obj) => changeRequests.filter((change) => change.config === configMain && (!args.state || change.state === args.state)),
  };
  const configPlatform: Obj = {
    id: 'config-platform',
    repository: 'acme/platform',
    repositoryId: '812004981',
    status: 'ACTIVE',
    root: 'tools/sanchoris/',
    branch: 'main',
    url: 'https://github.com/acme/platform/tree/main/tools/sanchoris',
    newSkillUrl: 'https://github.com/acme/platform/new/main/tools/sanchoris/skills',
    revision: { id: 'rev-12', number: 12, commitSha: '3b81aa0', committedAt: ago(60 * 24 * 4), validation: validOk },
    revisions: () => [],
    syncedAt: ago(2),
    syncError: null,
    triggerCount: 1,
    skillCount: 0,
    projects: () => [],
    changeRequests: () => [],
  };
  const configs = [configMain, configPlatform];

  const repo = (name: string, fullName: string, warmMinutes: number): Obj => ({
    id: `repo-${fullName}`,
    name,
    fullName,
    url: `https://github.com/${fullName}`,
    warmBuild: { succeeded: true, finishedAt: ago(warmMinutes) },
  });
  const shop: Obj = {
    id: 'project-shop',
    name: 'shop',
    config: configMain,
    systemPrompt: 'You are working on the Acme shop. Always write tests for new endpoints.',
    repositories: [repo('frontend', 'acme/shop-web', 12), repo('backend', 'acme/shop-api', 15)],
    warmBuild: { succeeded: true, finishedAt: ago(12) },
    sessions: () => sessions.filter((session) => session.project === shop),
  };
  const docs: Obj = {
    id: 'project-docs',
    name: 'docs',
    config: configMain,
    systemPrompt: null,
    repositories: [repo('site', 'acme/site', 180)],
    warmBuild: { succeeded: true, finishedAt: ago(180) },
    sessions: () => [],
  };
  projects.push(shop, docs);

  const teamCredential = credentials[1];
  const session = (name: string, project: Obj | null, repositories: string[], githubRepositories: string[]): Obj => ({
    id: `session-${name}`,
    name,
    agent: 'CLAUDE',
    model: 'sonnet-5-5',
    credentialName: 'acme-claude-team',
    credential: teamCredential,
    project,
    repositories,
    githubRepositories,
    compute: 'STANDARD',
    idleSeconds: 900,
    usageLimitResume: 'AUTO',
  });
  const shopSession = session('shop', shop, ['acme/shop-web', 'acme/shop-api'], ['acme/shop-web', 'acme/shop-api']);
  const allSession = session('all', null, ['acme/idea'], ['acme/*']);
  const depsSession = session('deps', shop, ['acme/shop-web', 'acme/shop-api'], ['acme/shop-web', 'acme/shop-api']);
  sessions.push(shopSession, allSession, depsSession);

  const skills: Obj[] = [];
  const skillRuns: Obj[] = [];
  const tasks: Obj[] = [];
  const triggerEvents: Obj[] = [];

  const activity = (running = 0, withAgent = 0, needsYou = 0, succeededToday = 0) => ({ running, withAgent, needsYou, succeededToday });
  const openPr: Obj = {
    id: 'skill-open-pr-green-ci',
    name: 'open-pr-green-ci',
    kind: 'RELIABLE',
    description: 'The change is ready: push, open pull requests, and get the checks green',
    comment: 'Push the branch, open pull requests, and hand CI failures back to the agent until the checks pass. Up to 3 fixes.',
    inputs: [
      { name: 'Repos', type: 'array' },
      { name: 'Issue', type: 'number' },
      { name: 'MaxFixes', type: 'number' },
    ],
    timeoutSeconds: 14400,
    skillMd: '---\nname: open-pr-green-ci\ndescription: The change is ready: push, open pull requests, and get the checks green\n---\n',
    skillYaml: 'Comment: Push the branch, open pull requests, and hand CI failures back to the agent until the checks pass\nStartAt: Init\n',
    startAt: 'Init',
    states: [
      { name: 'Init', type: 'PASS', resource: null, summary: 'fixes = 0', x: 0, y: 0, transitions: [{ to: 'Push', kind: 'NEXT', label: null }], activity: activity() },
      { name: 'Push', type: 'TASK', resource: 'sanchoris:::git:push', summary: 'git:push', x: 200, y: 0, transitions: [{ to: 'OpenPRs', kind: 'NEXT', label: null }], activity: activity() },
      { name: 'OpenPRs', type: 'TASK', resource: 'sanchoris:::github:createPullRequests', summary: 'createPullRequests', x: 400, y: 0, transitions: [{ to: 'WaitChecks', kind: 'NEXT', label: null }], activity: activity() },
      { name: 'WaitChecks', type: 'TASK', resource: 'sanchoris:::github:checks.waitForTaskToken', summary: 'checks.waitFor', x: 600, y: 0, transitions: [{ to: 'Green', kind: 'NEXT', label: null }], activity: activity(1) },
      {
        name: 'Green',
        type: 'CHOICE',
        resource: null,
        summary: 'checks pass?',
        x: 800,
        y: 0,
        transitions: [
          { to: 'Report', kind: 'CHOICE', label: 'success' },
          { to: 'GiveUp', kind: 'CHOICE', label: 'fixes ≥ 3' },
          { to: 'HandOff', kind: 'DEFAULT', label: 'else' },
        ],
        activity: activity(),
      },
      { name: 'Report', type: 'TASK', resource: 'sanchoris:::github:commentOnIssue', summary: 'commentOnIssue', x: 1000, y: 0, transitions: [], activity: activity(0, 0, 0, 2) },
      { name: 'CountFix', type: 'PASS', resource: null, summary: 'fixes + 1', x: 200, y: 250, transitions: [{ to: 'Push', kind: 'NEXT', label: null }], activity: activity() },
      { name: 'HandOff', type: 'TASK', resource: 'sanchoris:::agent:handoff.waitForTaskToken', summary: 'agent:handoff', x: 400, y: 250, transitions: [{ to: 'CountFix', kind: 'NEXT', label: null }], activity: activity(0, 1) },
      { name: 'GiveUp', type: 'FAIL', resource: null, summary: 'FixesExhausted', x: 1000, y: 250, transitions: [], activity: activity(0, 0, 1) },
    ],
    config: configMain,
    editUrl: 'https://github.com/acme/sanchoris-config/edit/main/skills/open-pr-green-ci/skill.yaml',
    stats: (args: Obj) => ({ days: args.days ?? 30, runs: 38, succeeded: 33, medianDurationSeconds: 22 * 60, medianHandoffs: 1, gaveUp: 4, failureErrors: ['Skill.FixesExhausted'] }),
    recentRuns: (args: Obj) =>
      skillRuns
        .filter((run) => run.skill === openPr)
        .sort((a, b) => b.startedAt.localeCompare(a.startedAt))
        .slice(0, args.limit ?? 10),
    requiredBy: () => triggers.filter((trigger) => trigger.requiredSkills.includes(openPr)),
    openChangeRequest: () => changeRequests.find((change) => change.state === 'OPEN' && change.changes.some((item: Obj) => item.kind === 'SKILL' && item.name === 'open-pr-green-ci')) ?? null,
  };
  const bump: Obj = {
    id: 'skill-bump-dependencies',
    name: 'bump-dependencies',
    kind: 'RELIABLE',
    description: 'Update the dependencies of a repository and verify the build',
    comment: 'Update the dependencies and run the build.',
    inputs: [{ name: 'Repo', type: 'string' }],
    timeoutSeconds: 3600,
    skillMd: '---\nname: bump-dependencies\n---\n',
    skillYaml: 'StartAt: Update\n',
    startAt: 'Update',
    states: [
      { name: 'Update', type: 'TASK', resource: 'sanchoris:::workspace:exec.sync', summary: 'pnpm update', x: 0, y: 0, transitions: [{ to: 'Build', kind: 'NEXT', label: null }], activity: activity() },
      { name: 'Build', type: 'TASK', resource: 'sanchoris:::workspace:exec.sync', summary: 'pnpm build', x: 200, y: 0, transitions: [{ to: 'Done', kind: 'NEXT', label: null }], activity: activity() },
      { name: 'Done', type: 'SUCCEED', resource: null, summary: 'build passes', x: 400, y: 0, transitions: [], activity: activity() },
    ],
    config: configMain,
    editUrl: 'https://github.com/acme/sanchoris-config/edit/main/skills/bump-dependencies/skill.yaml',
    stats: () => ({ days: 30, runs: 12, succeeded: 11, medianDurationSeconds: 9 * 60, medianHandoffs: 0, gaveUp: 0, failureErrors: [] }),
    recentRuns: () => [],
    requiredBy: () => [],
    openChangeRequest: () => null,
  };
  const writeTests: Obj = {
    id: 'skill-write-tests',
    name: 'write-tests',
    kind: 'PLAIN',
    description: 'How we write tests in the shop repositories',
    comment: null,
    inputs: [],
    timeoutSeconds: null,
    skillMd: '---\nname: write-tests\n---\n',
    skillYaml: null,
    startAt: null,
    states: [],
    config: configMain,
    editUrl: 'https://github.com/acme/sanchoris-config/edit/main/skills/write-tests/SKILL.md',
    stats: () => ({ days: 30, runs: 0, succeeded: 0, medianDurationSeconds: null, medianHandoffs: null, gaveUp: 0, failureErrors: [] }),
    recentRuns: () => [],
    requiredBy: () => [],
    openChangeRequest: () => null,
  };
  skills.push(openPr, bump, writeTests);

  const limits = (costUsd: number, timeoutSeconds: number) => ({ turns: 60, costUsd, timeoutSeconds, heartbeatSeconds: 900, childrenMaxDepth: 1, childrenMaxCount: 5 });
  const triggers: Obj[] = [];
  function trigger(spec: Obj): Obj {
    const item: Obj = {
      id: `trigger-${spec.name}`,
      instruction: 'Resolve issue <number> in <repository>: <title>. Write tests for the change.',
      skills: spec.requiredSkills,
      maxNudges: 2,
      limits: limits(20, 14400),
      config: configMain,
      editUrl: `https://github.com/acme/sanchoris-config/edit/main/triggers/${spec.name}.yaml`,
      yaml: `${spec.name}:\n  State: ${spec.state}\n`,
      activeTasks: () => tasks.filter((task) => task.trigger === item && !['SUCCEEDED', 'FAILED', 'ABORTED'].includes(task.status)).length,
      openChangeRequest: () => changeRequests.find((change) => change.state === 'OPEN' && change.changes.some((ref: Obj) => ref.kind === 'TRIGGER' && ref.name === spec.name)) ?? null,
      events: (args: Obj) => triggerEvents.filter((event) => event.trigger === item).slice(0, args.limit ?? 20),
      ...spec,
    };
    triggers.push(item);
    return item;
  }
  const match = (spec: Obj) => ({ repositories: [], labels: [], senders: [], chatScope: null, slackChannel: null, scheduleExpression: null, nextRunAt: null, ...spec });
  const labeledIssue = trigger({
    name: 'labeled-issue',
    state: 'ENABLED',
    match: match({ source: 'GITHUB', kind: 'ISSUE_LABELED', repositories: ['acme/shop-web', 'acme/shop-api'], labels: ['sanchoris'], senders: ['mika-dev', 'sota-dev', 'sanchoris[bot]'] }),
    session: shopSession,
    requiredSkills: [openPr],
    tasksLast7d: 9,
  });
  const chatAll = trigger({
    name: 'chat-all',
    state: 'ENABLED',
    match: match({ source: 'WEB', kind: 'MESSAGE', chatScope: 'all' }),
    session: allSession,
    instruction: '<text>',
    requiredSkills: [],
    maxNudges: 0,
    limits: limits(10, 86400),
    tasksLast7d: 5,
  });
  const chatShop = trigger({
    name: 'chat-shop',
    state: 'ENABLED',
    match: match({ source: 'WEB', kind: 'MESSAGE', chatScope: 'shop' }),
    session: shopSession,
    instruction: '<text>',
    requiredSkills: [],
    skills: [openPr],
    maxNudges: 0,
    limits: limits(10, 86400),
    tasksLast7d: 1,
  });
  const weeklyDeps = trigger({
    name: 'weekly-deps',
    state: 'ENABLED',
    match: match({ source: 'SCHEDULE', kind: 'SCHEDULE', scheduleExpression: 'cron(0 9 ? * MON *)', nextRunAt: nextMonday9() }),
    session: depsSession,
    requiredSkills: [openPr],
    tasksLast7d: 1,
  });
  trigger({
    name: 'failed-ci',
    state: 'DISABLED',
    match: match({ source: 'GITHUB', kind: 'CHECK_RUN_COMPLETED', repositories: ['acme/shop-web', 'acme/shop-api'] }),
    session: shopSession,
    requiredSkills: [openPr],
    config: configPlatform,
    tasksLast7d: 0,
  });

  changeRequests.push({
    id: 'cr-7',
    config: configMain,
    number: 7,
    title: 'Add a docs step to open-pr-green-ci',
    url: 'https://github.com/acme/sanchoris-config/pull/7',
    state: 'OPEN',
    changes: [
      { kind: 'SKILL', name: 'open-pr-green-ci' },
      { kind: 'TRIGGER', name: 'labeled-issue' },
    ],
    validation: validOk,
    openedFromTask: () => tasks.find((task) => task.id === 'task-chat-export') ?? null,
    openedBy: mika,
    openedAt: ago(60 * 3),
  });

  const pr = (repository: string, number: number, state: string, minutes: number): Obj => ({
    id: `output-${repository}-${number}`,
    kind: 'PULL_REQUEST',
    repository,
    number,
    url: `https://github.com/${repository}/pull/${number}`,
    state,
    createdAt: ago(minutes),
  });

  function issue(repository: string, number: number, title: string, task?: () => Obj | undefined): Obj {
    return { repository, number, title, url: `https://github.com/${repository}/issues/${number}`, task: () => task?.() ?? null };
  }

  function makeTask(spec: Obj): Obj {
    const item: Obj = {
      name: spec.id,
      waitReason: null,
      pendingReason: null,
      needsYouReason: null,
      resumeAt: null,
      autoResume: true,
      snoozedUntil: null,
      nudges: 0,
      error: null,
      cause: null,
      endedAt: null,
      triggerEvent: null,
      requester: null,
      configRevision: 41,
      issue: null,
      channel: null,
      parent: null,
      blockedBy: [],
      runs: [],
      outputs: [],
      messages: [],
      turns: 0,
      costUsd: 0,
      ...spec,
    };
    item.session = spec.session ?? spec.trigger.session;
    item.maxNudges = spec.trigger.maxNudges;
    item.limits = spec.trigger.limits;
    item.children = () => tasks.filter((task) => task.parent === item);
    item.filedTasks = () => {
      const roots = tasks.filter((task) => task.requester?.task === item);
      return roots.flatMap((root) => [root, ...tasks.filter((task) => task.parent === root)]);
    };
    item.skillRuns = () => skillRuns.filter((run) => run.task === item).sort((a, b) => a.number - b.number);
    item.requiredSkills = () =>
      item.trigger.requiredSkills.map((skill: Obj) => {
        const runs = skillRuns.filter((run) => run.task === item && run.skill === skill);
        const latestRun = runs[runs.length - 1] ?? null;
        const state = !latestRun
          ? 'NOT_RUN'
          : latestRun.status === 'SUCCEEDED'
            ? 'SUCCEEDED'
            : latestRun.status === 'FAILED'
              ? 'FAILED'
              : 'RUNNING';
        return { skill, state, latestRun };
      });
    item.run = (args: Obj) => item.runs.find((run: Obj) => run.number === args.number) ?? null;
    tasks.push(item);
    return item;
  }

  function makeRun(task: Obj, spec: Obj): Obj {
    const run: Obj = {
      id: `run-${task.id}-${spec.number ?? 1}`,
      number: 1,
      task,
      sessionName: task.session.name,
      agent: task.session.agent,
      model: task.session.model,
      credentialName: task.session.credentialName,
      agentSessionId: `sess_${task.id}`,
      endedAt: null,
      summary: null,
      turns: task.turns,
      costUsd: task.costUsd,
      outputs: task.outputs,
      lines: [],
      ...spec,
    };
    run.workspace = {
      platform: 'Fargate',
      compute: 'STANDARD',
      architecture: 'ARM64',
      capacityProvider: 'FARGATE_SPOT',
      vcpu: 2,
      memoryMib: 4096,
      repositories: task.session.project ? task.session.project.repositories.map((repository: Obj) => repository.name) : task.session.repositories,
      branch: spec.branch ?? `agent/${task.issue?.number ?? task.id}`,
      baseCommit: 'a1b2c3d',
    };
    run.log = (args: Obj) => run.lines.filter((line: Obj) => args.after == null || line.sequence > args.after);
    run.skillRuns = () => skillRuns.filter((skillRun) => skillRun.run === run);
    task.runs.push(run);
    return run;
  }

  function makeSkillRun(spec: Obj): Obj {
    const run: Obj = {
      id: `skillrun-${spec.task.id}-${spec.number ?? 1}`,
      number: 1,
      input: JSON.stringify({ Repos: [spec.task.issue?.repository], Issue: spec.task.issue?.number, MaxFixes: 3 }),
      output: null,
      error: null,
      cause: null,
      endedAt: null,
      entries: [],
      outputs: spec.task.outputs,
      ...spec,
    };
    run.stateEntryCount = () => run.entries.length;
    run.stateEntries = (args: Obj) => (args.last ? run.entries.slice(-args.last) : run.entries);
    skillRuns.push(run);
    return run;
  }

  function entries(skillRunId: string, start: number, rows: [string, string, string, string | null][]): Obj[] {
    return rows.map(([stateName, stateType, status, result], index) => ({
      id: `${skillRunId}-entry-${index + 1}`,
      sequence: index + 1,
      stateName,
      stateType,
      status,
      enteredAt: ago(start - index * 4),
      exitedAt: ago(start - index * 4 - 1),
      attempts: 1,
      input: null,
      output: null,
      error: status === 'FAILED' ? 'Skill.FixesExhausted' : null,
      cause: null,
      nextState: null,
      result,
    }));
  }

  const base35 = 190;
  const chatExport = makeTask({
    id: 'task-chat-export',
    title: 'Bulk export for orders',
    status: 'WAITING',
    waitReason: 'CHAT',
    trigger: chatAll,
    createdAt: ago(40),
    startedAt: ago(40),
    updatedAt: ago(8),
    channel: { kind: 'WEB', threadId: 'web-thread-1', scope: 'all', slackChannel: null },
    requester: { kind: 'MEMBER', member: mika, task: null, viaChat: true },
    turns: 9,
    costUsd: 2.05,
  });
  const t35 = makeTask({
    id: 'task-35',
    title: 'Fix flaky checkout test',
    status: 'NEEDS_YOU',
    needsYouReason: 'SKILL_FAILED',
    error: 'Skill.FixesExhausted',
    cause: 'The checks still fail after 3 fixes: checkout_spec::flaky.',
    trigger: labeledIssue,
    createdAt: ago(base35),
    startedAt: ago(base35),
    updatedAt: ago(68),
    issue: issue('acme/shop-web', 35, 'Fix flaky checkout test', () => t35),
    requester: { kind: 'MEMBER', member: mika, task: null, viaChat: true },
    turns: 52,
    costUsd: 9.2,
    outputs: [pr('acme/shop-web', 52, 'OPEN', base35 - 38)],
  });
  t35.triggerEvent = { id: 'event-35', receivedAt: ago(base35), senderLogin: 'mika-dev' };
  const t38 = makeTask({
    id: 'task-38',
    title: 'Add bulk export to the orders page',
    status: 'RUNNING',
    trigger: labeledIssue,
    createdAt: ago(35),
    startedAt: ago(35),
    updatedAt: ago(2),
    issue: issue('acme/shop-web', 38, 'Add bulk export', () => t38),
    requester: { kind: 'AGENT', member: null, task: chatExport, viaChat: false },
    turns: 18,
    costUsd: 4.1,
  });
  t38.triggerEvent = { id: 'event-38', receivedAt: ago(35), senderLogin: 'sanchoris[bot]' };
  const t41 = makeTask({
    id: 'task-41',
    title: 'Orders export endpoint',
    status: 'RUNNING',
    trigger: labeledIssue,
    createdAt: ago(20),
    startedAt: ago(20),
    updatedAt: ago(0),
    issue: issue('acme/shop-api', 41, 'Orders export endpoint', () => t41),
    parent: t38,
    requester: { kind: 'AGENT', member: null, task: t38, viaChat: false },
    turns: 35,
    costUsd: 3.42,
    outputs: [pr('acme/shop-api', 53, 'OPEN', 6)],
  });
  t41.triggerEvent = { id: 'event-41', receivedAt: ago(20), senderLogin: 'sanchoris[bot]' };
  const t42 = makeTask({
    id: 'task-42',
    title: 'Export button and CSV download',
    status: 'RUNNING',
    trigger: labeledIssue,
    createdAt: ago(20),
    startedAt: ago(20),
    updatedAt: ago(1),
    issue: issue('acme/shop-web', 42, 'Export button and CSV download', () => t42),
    parent: t38,
    requester: { kind: 'AGENT', member: null, task: t38, viaChat: false },
    turns: 13,
    costUsd: 1.65,
    outputs: [pr('acme/shop-web', 54, 'OPEN', 9)],
  });
  const t43 = makeTask({
    id: 'task-43',
    title: 'Docs for the export API',
    status: 'PENDING',
    pendingReason: 'BLOCKED',
    trigger: labeledIssue,
    createdAt: ago(20),
    startedAt: null,
    updatedAt: ago(5),
    issue: issue('acme/shop-api', 43, 'Docs for the export API', () => t43),
    parent: t38,
    blockedBy: [t41],
    requester: { kind: 'AGENT', member: null, task: t38, viaChat: false },
  });
  makeTask({
    id: 'task-44',
    title: 'Upgrade Node to 24',
    status: 'WAITING',
    waitReason: 'USAGE_LIMIT',
    resumeAt: nextMonday9(),
    autoResume: true,
    trigger: labeledIssue,
    createdAt: ago(60 * 20),
    startedAt: ago(60 * 20),
    updatedAt: ago(60 * 3),
    issue: issue('acme/shop-web', 44, 'Upgrade Node to 24'),
    requester: { kind: 'MEMBER', member: sota, task: null, viaChat: false },
    turns: 22,
    costUsd: 2.4,
  });
  const t33 = makeTask({
    id: 'task-33',
    title: 'Bump dependencies',
    status: 'SUCCEEDED',
    trigger: weeklyDeps,
    createdAt: ago(60 * 3 + 20),
    startedAt: ago(60 * 3 + 20),
    endedAt: ago(60 * 3),
    updatedAt: ago(60 * 3),
    issue: issue('acme/shop-api', 33, 'Bump dependencies'),
    turns: 8,
    costUsd: 0.88,
    outputs: [pr('acme/shop-api', 50, 'MERGED', 60 * 3 + 6)],
  });
  const t31 = makeTask({
    id: 'task-31',
    title: 'Fix typo in checkout copy',
    status: 'SUCCEEDED',
    trigger: labeledIssue,
    createdAt: ago(60 * 26),
    startedAt: ago(60 * 26),
    endedAt: ago(60 * 25),
    updatedAt: ago(60 * 25),
    issue: issue('acme/shop-web', 31, 'Fix typo in checkout copy'),
    requester: { kind: 'MEMBER', member: mika, task: null, viaChat: false },
    turns: 4,
    costUsd: 0.41,
    outputs: [pr('acme/shop-web', 49, 'MERGED', 60 * 25 + 8)],
  });
  const chatFlaky = makeTask({
    id: 'task-chat-flaky',
    title: 'Fix flaky checkout test',
    status: 'SUCCEEDED',
    trigger: chatShop,
    createdAt: ago(base35 + 15),
    startedAt: ago(base35 + 15),
    endedAt: ago(base35 + 2),
    updatedAt: ago(base35 + 2),
    channel: { kind: 'SLACK', threadId: 'slack-thread-1', scope: 'shop', slackChannel: '#shop-dev' },
    requester: { kind: 'MEMBER', member: mika, task: null, viaChat: true },
    turns: 3,
    costUsd: 0.31,
  });
  const chatNode = makeTask({
    id: 'task-chat-node',
    title: 'Upgrade Node to 24',
    status: 'SUCCEEDED',
    trigger: chatShop,
    createdAt: ago(60 * 21),
    startedAt: ago(60 * 21),
    endedAt: ago(60 * 20),
    updatedAt: ago(60 * 20),
    channel: { kind: 'SLACK', threadId: 'slack-thread-2', scope: 'shop', slackChannel: '#shop-dev' },
    requester: { kind: 'MEMBER', member: sota, task: null, viaChat: true },
    turns: 2,
    costUsd: 0.22,
  });
  t35.requester = { kind: 'MEMBER', member: mika, task: chatFlaky, viaChat: true };
  const t44 = tasks.find((task) => task.id === 'task-44') as Obj;
  t44.requester = { kind: 'MEMBER', member: sota, task: chatNode, viaChat: true };

  const run35 = makeRun(t35, { status: 'IDLE', startedAt: ago(base35), branch: 'agent/35', credentialName: 'acme-claude-team' });
  const at35 = (offset: number) => ago(base35 - offset);
  run35.lines = [
    { kind: 'SYSTEM', offset: 0.03, text: 'workspace created · Fargate 2 vCPU · warm cache restored (frontend)' },
    { kind: 'SYSTEM', offset: 0.18, text: 'session started · credential: acme-claude-team' },
    {
      kind: 'INSTRUCTION',
      offset: 0.2,
      text: 'Trigger labeled-issue started this task: Fix flaky checkout test (acme/shop-web#35).\nInstruction: Resolve issue 35 in acme/shop-web.\nSkills: open-pr-green-ci (required). Run it with `sanchoris skill run open-pr-green-ci`.',
    },
    { kind: 'AGENT', offset: 1.6, text: 'Reading the checkout tests. The cart total is rounded twice.' },
    { kind: 'AGENT', offset: 37.3, text: 'Fixed the rounding and added a test. Running the skill.' },
    { kind: 'TOOL', offset: 38, text: 'sanchoris skill run open-pr-green-ci --input {"Repos":["acme/shop-web"],"Issue":35,"MaxFixes":3}' },
    { kind: 'EVENT', offset: 38.02, text: 'started skill run 1 (RUNNING). You can end the turn; sanchoris will message you.' },
    { kind: 'AGENT', offset: 38.1, text: 'The skill is running. Waiting for its result.' },
    { kind: 'EVENT', offset: 38.15, text: 'session idle · the workspace is released after 900s' },
    { kind: 'INSTRUCTION', offset: 71, text: 'Skill open-pr-green-ci (run 1) needs you: CI failed on acme/shop-web#52.\ncheckout_spec::flaky. Fix it, commit, and run `sanchoris skill continue 1`.' },
    { kind: 'EVENT', offset: 71.1, text: 'workspace restored from the session record and branch agent/35' },
    { kind: 'AGENT', offset: 78, text: 'Made the clock injectable in the test. Continuing the skill.' },
    { kind: 'TOOL', offset: 78.02, text: 'sanchoris skill continue 1' },
    { kind: 'INSTRUCTION', offset: 95.5, text: 'Skill open-pr-green-ci (run 1) needs you: CI failed again. Fix 3 of 3.' },
    { kind: 'AGENT', offset: 103, text: 'Still flaky in CI. Continuing the skill.' },
    { kind: 'ERROR', offset: 121.7, text: '[sanchoris] Skill open-pr-green-ci (run 1) failed: Skill.FixesExhausted (3 fixes).' },
    { kind: 'EVENT', offset: 121.72, text: 'session idle · the workspace is released after 900s · the task needs you' },
  ].map((line, index) => ({ sequence: index + 1, at: at35(line.offset), kind: line.kind, text: line.text }));

  const sr35 = makeSkillRun({ task: t35, skill: openPr, run: run35, status: 'FAILED', currentState: 'GiveUp', handoffs: 3, startedAt: at35(38), endedAt: at35(121.7), error: 'Skill.FixesExhausted', cause: 'checks fail after 3 fixes' });
  sr35.entries = entries(sr35.id, base35 - 38, [
    ['Init', 'PASS', 'SUCCEEDED', 'fixes = 0'],
    ['Push', 'TASK', 'SUCCEEDED', 'agent/35 pushed'],
    ['OpenPRs', 'TASK', 'SUCCEEDED', 'acme/shop-web#52 opened'],
    ['WaitChecks', 'TASK', 'SUCCEEDED', 'checks: failure · checkout_spec::flaky'],
    ['Green', 'CHOICE', 'SUCCEEDED', 'else → HandOff'],
    ['HandOff', 'TASK', 'SUCCEEDED', 'fix 1 · agent continued'],
    ['CountFix', 'PASS', 'SUCCEEDED', 'fixes = 1'],
    ['Push', 'TASK', 'SUCCEEDED', 'agent/35 pushed'],
    ['WaitChecks', 'TASK', 'SUCCEEDED', 'checks: failure · checkout_spec::flaky'],
    ['Green', 'CHOICE', 'SUCCEEDED', 'else → HandOff'],
    ['HandOff', 'TASK', 'SUCCEEDED', 'fix 2 · agent continued'],
    ['CountFix', 'PASS', 'SUCCEEDED', 'fixes = 2'],
    ['Push', 'TASK', 'SUCCEEDED', 'agent/35 pushed'],
    ['WaitChecks', 'TASK', 'SUCCEEDED', 'checks: failure · checkout_spec::flaky'],
    ['Green', 'CHOICE', 'SUCCEEDED', 'else → HandOff'],
    ['HandOff', 'TASK', 'SUCCEEDED', 'fix 3 · agent continued'],
    ['CountFix', 'PASS', 'SUCCEEDED', 'fixes = 3'],
    ['Push', 'TASK', 'SUCCEEDED', 'agent/35 pushed'],
    ['WaitChecks', 'TASK', 'SUCCEEDED', 'checks: failure · checkout_spec::flaky'],
    ['Green', 'CHOICE', 'SUCCEEDED', 'fixes ≥ 3 → GiveUp'],
    ['GiveUp', 'FAIL', 'FAILED', 'Skill.FixesExhausted · checks fail after 3 fixes'],
  ]);

  const simpleLog = (task: Obj, run: Obj, lines: [string, number, string][]) => {
    run.lines = lines.map(([kind, minutes, text], index) => ({ sequence: index + 1, at: ago(minutes), kind, text }));
    return task;
  };
  const run38 = makeRun(t38, { status: 'RUNNING', startedAt: ago(35) });
  simpleLog(t38, run38, [
    ['SYSTEM', 35, 'workspace created · Fargate 2 vCPU · warm cache restored (frontend)'],
    ['INSTRUCTION', 35, 'Trigger labeled-issue started this task: Add bulk export to the orders page (acme/shop-web#38).\nSkills: open-pr-green-ci (required).'],
    ['AGENT', 30, 'The API and the UI are separate. Filing #41, #42 and #43 as sub-issues.'],
    ['TOOL', 29, 'gh issue create --repo acme/shop-api --title "Orders export endpoint"'],
    ['AGENT', 2, 'Waiting for the child tasks before wiring the page.'],
  ]);
  const run41 = makeRun(t41, { status: 'IDLE', startedAt: ago(20) });
  simpleLog(t41, run41, [
    ['SYSTEM', 20, 'workspace created · Fargate 2 vCPU'],
    ['INSTRUCTION', 20, 'Trigger labeled-issue started this task: Orders export endpoint (acme/shop-api#41).'],
    ['TOOL', 7, 'sanchoris skill run open-pr-green-ci --input {"Repos":["acme/shop-api"],"Issue":41,"MaxFixes":3}'],
    ['EVENT', 7, 'started skill run 1 (RUNNING). You can end the turn; sanchoris will message you.'],
  ]);
  const run42 = makeRun(t42, { status: 'RUNNING', startedAt: ago(20) });
  simpleLog(t42, run42, [
    ['SYSTEM', 20, 'workspace created · Fargate 2 vCPU'],
    ['INSTRUCTION', 20, 'Trigger labeled-issue started this task: Export button and CSV download (acme/shop-web#42).'],
    ['INSTRUCTION', 3, 'Skill open-pr-green-ci (run 1) needs you: CI failed on acme/shop-web#54.\nexport_spec. Fix it, commit, and run `sanchoris skill continue 1`.'],
    ['AGENT', 1, 'The CSV header is missing. Fixing.'],
  ]);
  const run44 = makeRun(t44, { status: 'IDLE', startedAt: ago(60 * 20) });
  simpleLog(t44, run44, [
    ['SYSTEM', 60 * 20, 'workspace created · Fargate 2 vCPU'],
    ['ERROR', 60 * 3, 'Worker.UsageLimit: the Claude limit is reached. Resumes at the reset.'],
  ]);
  const run33 = makeRun(t33, { status: 'COMPLETED', startedAt: ago(60 * 3 + 20), endedAt: ago(60 * 3) });
  simpleLog(t33, run33, [['SYSTEM', 60 * 3 + 20, 'workspace created · Fargate 2 vCPU']]);
  const run31 = makeRun(t31, { status: 'COMPLETED', startedAt: ago(60 * 26), endedAt: ago(60 * 25) });
  simpleLog(t31, run31, [['SYSTEM', 60 * 26, 'workspace created · Fargate 2 vCPU']]);
  const runChat = makeRun(chatExport, { status: 'IDLE', startedAt: ago(40), branch: 'agent/chat-1' });
  simpleLog(chatExport, runChat, [
    ['SYSTEM', 40, 'workspace created · Fargate 2 vCPU · acme/idea'],
    ['USER', 30, 'We need CSV export on the orders page. Backend and frontend are separate repos.'],
  ]);
  makeRun(chatFlaky, { status: 'COMPLETED', startedAt: ago(base35 + 15), endedAt: ago(base35 + 2) });
  makeRun(chatNode, { status: 'COMPLETED', startedAt: ago(60 * 21), endedAt: ago(60 * 20) });

  makeSkillRun({ task: t41, skill: openPr, run: run41, status: 'RUNNING', currentState: 'WaitChecks', handoffs: 0, startedAt: ago(7) }).entries = entries('sr41', 7, [
    ['Init', 'PASS', 'SUCCEEDED', 'fixes = 0'],
    ['Push', 'TASK', 'SUCCEEDED', 'agent/41 pushed'],
    ['OpenPRs', 'TASK', 'SUCCEEDED', 'acme/shop-api#53 opened'],
    ['WaitChecks', 'TASK', 'RUNNING', 'waiting for 4 checks'],
  ]);
  makeSkillRun({ task: t42, skill: openPr, run: run42, status: 'WAITING_AGENT', currentState: 'HandOff', handoffs: 1, startedAt: ago(14) }).entries = entries('sr42', 14, [
    ['Init', 'PASS', 'SUCCEEDED', 'fixes = 0'],
    ['Push', 'TASK', 'SUCCEEDED', 'agent/42 pushed'],
    ['OpenPRs', 'TASK', 'SUCCEEDED', 'acme/shop-web#54 opened'],
    ['WaitChecks', 'TASK', 'SUCCEEDED', 'checks: failure · export_spec'],
    ['Green', 'CHOICE', 'SUCCEEDED', 'else → HandOff'],
    ['HandOff', 'TASK', 'WAITING', 'fix 1 · with the agent'],
  ]);
  makeSkillRun({ task: t33, skill: openPr, run: run33, status: 'SUCCEEDED', currentState: 'Report', handoffs: 1, startedAt: ago(60 * 3 + 14), endedAt: ago(60 * 3) });
  makeSkillRun({ task: t31, skill: openPr, run: run31, status: 'SUCCEEDED', currentState: 'Report', handoffs: 0, startedAt: ago(60 * 25 + 6), endedAt: ago(60 * 25) });

  const message = (id: string, authorKind: string, member: Obj | null, minutes: number, text: string, issues: Obj[] = []): Obj => ({ id, authorKind, member, at: ago(minutes), text, issues });
  chatExport.messages = [
    message('msg-1', 'MEMBER', mika, 30, 'We need CSV export on the orders page. Backend and frontend are separate repos.'),
    message('msg-2', 'AGENT', null, 29, 'I can split this into three issues. The API comes first, then the UI in parallel, and docs after the API lands.', [
      issue('acme/shop-api', 41, 'Orders export endpoint', () => t41),
      issue('acme/shop-web', 42, 'Export button and CSV download', () => t42),
      issue('acme/shop-api', 43, 'Docs for the export API', () => t43),
    ]),
    message('msg-3', 'MEMBER', mika, 27, 'Looks right. Go.'),
    message('msg-4', 'AGENT', null, 26, 'Created #41, #42 and #43 as children of #38. Two agents are working now. I will post here when the pull requests are open.'),
  ];
  chatFlaky.messages = [
    message('msg-f1', 'MEMBER', mika, base35 + 15, 'The checkout test is flaky again. Can you take it?'),
    message('msg-f2', 'AGENT', null, base35 + 3, 'Filed it as an issue. A task works on it now.', [issue('acme/shop-web', 35, 'Fix flaky checkout test', () => t35)]),
  ];
  chatNode.messages = [
    message('msg-n1', 'MEMBER', sota, 60 * 21, 'Please upgrade shop-web to Node 24.'),
    message('msg-n2', 'AGENT', null, 60 * 20, 'Filed #44. The task will open a pull request.', [issue('acme/shop-web', 44, 'Upgrade Node to 24', () => t44)]),
  ];
  chatExport.filedTasks = () => [t38, t41, t42, t43];
  chatFlaky.filedTasks = () => [t35];
  chatNode.filedTasks = () => [t44];

  const event = (id: string, minutes: number, triggerItem: Obj, result: string, extra: Obj = {}): Obj => ({
    id,
    receivedAt: ago(minutes),
    trigger: triggerItem,
    result,
    reason: null,
    senderLogin: null,
    task: null,
    limitedTask: null,
    idempotencyKey: id,
    ...extra,
  });
  triggerEvents.push(
    event('ev-1', 20, labeledIssue, 'STARTED', { task: t41, senderLogin: 'sanchoris[bot]' }),
    event('ev-2', 35, labeledIssue, 'STARTED', { task: t38, senderLogin: 'sanchoris[bot]' }),
    event('ev-3', 35, labeledIssue, 'SKIPPED', { reason: 'CHILD_LIMIT', limitedTask: t38, senderLogin: 'sanchoris[bot]' }),
    event('ev-4', 40, chatAll, 'STARTED', { task: chatExport, senderLogin: 'mika-dev' }),
    event('ev-5', 60 * 24 * 2, labeledIssue, 'SKIPPED', { reason: 'SENDER_NOT_ALLOWED', senderLogin: 'tanaka-dev' }),
    event('ev-6', 60 * 24 * 3, labeledIssue, 'DISCARDED', { reason: 'RECEIVE_LIMIT' }),
  );

  const audit = (id: string, minutes: number, actorKind: string, actorName: string, action: string, target: string, summary: string | null, origin: string, ip: string | null, targetTask: Obj | null = null, member: Obj | null = null): Obj => ({
    id,
    at: ago(minutes),
    actorKind,
    actorName,
    action,
    target,
    summary,
    origin,
    ip,
    targetTask,
    member,
  });
  const auditEvents: Obj[] = [
    audit('audit-1', 70, 'MEMBER', 'Mika', 'task.resume', '#35 Fix flaky checkout test', 'resumed at Work with a message', 'WEB', '203.0.113.8', t35, mika),
    audit('audit-2', 20, 'TRIGGER', 'labeled-issue', 'task.start', '#41 Orders export endpoint', 'issue-to-pr', 'GITHUB', null, t41),
    audit('audit-3', 21, 'AGENT', 'Run 1 · #38', 'issue.create', 'shop-api#41', 'filed by the agent, sub-issue of #38', 'GITHUB', null, t41),
    audit('audit-4', 60 * 3, 'MEMBER', 'Sota', 'changerequest.merge', 'sanchoris-config#6', 'adds the weekly-deps trigger', 'WEB', '198.51.100.4', null, sota),
    audit('audit-5', 60 * 6, 'MEMBER', 'Mika', 'member.invite', 'ren@acme.dev', 'member', 'WEB', '203.0.113.8', null, mika),
    audit('audit-6', 60 * 22, 'MEMBER', 'Mika', 'credential.disconnect', 'mika-codex', 'login rejected', 'WEB', '203.0.113.8', null, mika),
    audit('audit-7', 60 * 30, 'SYSTEM', 'system', 'trigger.discard', 'labeled-issue', '2 events over the receive limit', 'SYSTEM', null),
    audit('audit-8', 60 * 48, 'MEMBER', 'Mika', 'plan.change', 'Starter → Pro', null, 'WEB', '203.0.113.8', null, mika),
  ].sort((a, b) => b.at.localeCompare(a.at));

  Object.assign(tenant, {
    usage: periodUsage(31, 142, 8, 3, 'MONTH'),
    viewerMember: mika,
    setup: {
      githubLinked: true,
      appInstalled: true,
      configAdded: true,
      credentialConnected: true,
      triggerEnabled: true,
      firstTaskStarted: true,
      projectCreated: true,
      slackConnected: false,
      complete: true,
    },
    members: () => members,
    memberList: members,
    invitationList: invitations,
    invitations: (args: Obj) => invitations.filter((invitation) => !args.state || invitation.state === args.state),
    integrations: () => integrations,
    credentialList: credentials,
    credentials: () => credentials.filter((credential) => !credential.hidden),
    configs: () => configs,
    projects: () => projects,
    sessions: () => sessions,
    changeRequestList: changeRequests,
    changeRequests: (args: Obj) => changeRequests.filter((change) => !args.state || change.state === args.state),
    triggerList: triggers,
    triggers: () => triggers,
    triggerEvents: (args: Obj) => triggerEvents.slice(0, args.limit ?? 20),
    skills: () => skills,
    skill: (args: Obj) => skills.find((skill) => skill.name === args.name) ?? null,
    taskList: tasks,
    tasks: (args: Obj) => {
      const filter = args.filter ?? {};
      return tasks.filter((task) => {
        if (filter.statuses && !filter.statuses.includes(task.status)) return false;
        if (filter.active && ['SUCCEEDED', 'FAILED', 'ABORTED'].includes(task.status)) return false;
        if (filter.triggerId && task.trigger.id !== filter.triggerId) return false;
        if (filter.skillName && !task.trigger.requiredSkills.some((skill: Obj) => skill.name === filter.skillName)) return false;
        return true;
      });
    },
    task: (args: Obj) => tasks.find((task) => task.id === args.id) ?? null,
    chats: () => tasks.filter((task) => task.channel).sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)),
    auditEvents: (args: Obj) => {
      const filter = args.filter ?? {};
      return auditEvents
        .filter((item) => (!filter.since || item.at >= filter.since) && (!filter.action || item.action === filter.action) && (!filter.memberId || item.member?.id === filter.memberId) && (!filter.actorKind || item.actorKind === filter.actorKind))
        .slice(0, args.limit ?? 100);
    },
    auditList: auditEvents,
    sessionByScope: (scope: string) => triggers.find((item) => item.match.chatScope === scope && item.state === 'ENABLED'),
  });
  return tenant;
}


export function buildSetupTenant(spec: { slug: string; name: string; planName: string; role: string; installed: boolean }): Obj {
  const { slug, name, installed } = spec;
  const plan = planByName(spec.planName);
  const tenant: Obj = {
    id: `tenant-${slug}`,
    slug,
    name,
    plan,
    state: 'ACTIVE',
    suspendedReason: null,
    createdAt: ago(installed ? 60 * 24 * 2 : 0),
    githubOwner: installed ? { kind: 'ORGANIZATION', login: `${slug}-dev` } : null,
    githubAppInstallUrl: 'https://github.com/apps/sanchoris/installations/new',
    activity: { runningTasks: 0, runsToday: 0, costTodayUsd: 0, pullRequestsOpenedToday: 0, runsLast24h: Array.from({ length: 24 }, () => 0) },
    usage: periodUsage(installed ? 8 : 0, installed ? 31 : 0, 1, 0, plan.limitPeriod),
  };
  const member: Obj = {
    id: `member-mika-${slug}`,
    user: users.mika,
    tenant: () => tenant,
    role: spec.role,
    joinedAt: ago(0),
    lastActiveAt: ago(0),
    tasksStarted: 0,
    credentials: () => [],
  };
  const configs: Obj[] = installed
    ? [
        {
          id: `config-${slug}`,
          repository: `${slug}-dev/sanchoris-config`,
          repositoryId: '812005120',
          status: 'ACTIVE',
          root: '/',
          branch: 'main',
          url: `https://github.com/${slug}-dev/sanchoris-config`,
          newSkillUrl: `https://github.com/${slug}-dev/sanchoris-config/new/main/skills`,
          revision: { id: `rev-${slug}-1`, number: 1, commitSha: '0c1d2e3', committedAt: ago(60), validation: validOk },
          revisions: () => [],
          syncedAt: ago(3),
          syncError: null,
          triggerCount: 0,
          skillCount: 0,
          projects: () => [],
          changeRequests: () => [],
        },
      ]
    : [];
  const integrations: Obj[] = [
    installed
      ? {
          id: `int-${slug}-github`,
          kind: 'GITHUB_INSTALLATION',
          status: 'CONNECTED',
          account: `${slug}-dev`,
          repositories: [`${slug}-dev/app`, `${slug}-dev/api`, `${slug}-dev/sanchoris-config`],
          manageUrl: `https://github.com/organizations/${slug}-dev/settings/installations`,
          lastSyncedAt: ago(3),
          error: null,
        }
      : { id: `int-${slug}-github`, kind: 'GITHUB_INSTALLATION', status: 'NOT_CONNECTED', account: null, repositories: [], manageUrl: null, lastSyncedAt: null, error: null },
    { id: `int-${slug}-slack`, kind: 'SLACK', status: 'NOT_CONNECTED', account: null, repositories: [], manageUrl: null, lastSyncedAt: null, error: null },
  ];
  const credentials: Obj[] = [];
  Object.assign(tenant, {
    viewerMember: member,
    setup: {
      githubLinked: !!users.mika.githubLogin,
      appInstalled: installed,
      configAdded: configs.length > 0,
      get credentialConnected() {
        return credentials.length > 0;
      },
      triggerEnabled: false,
      firstTaskStarted: false,
      projectCreated: false,
      slackConnected: false,
      complete: false,
    },
    members: () => [member],
    memberList: [member],
    invitationList: [],
    invitations: () => [],
    integrations: () => integrations,
    credentialList: credentials,
    credentials: () => credentials,
    configs: () => configs,
    projects: () => [],
    sessions: () => [],
    changeRequestList: [],
    changeRequests: () => [],
    triggerList: [],
    triggers: () => [],
    triggerEvents: () => [],
    skills: () => [],
    skill: () => null,
    taskList: [],
    tasks: () => [],
    task: () => null,
    chats: () => [],
    auditEvents: () => [],
    auditList: [],
  });
  return tenant;
}

function operatorOnly(id: string, name: string, planName: string, owner: Obj, usage: Obj, state = 'ACTIVE', suspendedReason: string | null = null): Obj {
  return { id, slug: name, name, plan: planByName(planName), state, suspendedReason, githubOwner: owner, usage };
}

export function buildFixtures() {
  const acme = buildAcme();
  const northwind = buildSetupTenant({ slug: 'northwind', name: 'northwind', planName: 'Starter', role: 'ADMIN', installed: true });
  const tenants = [acme, northwind];
  const memberships = [acme.viewerMember, northwind.viewerMember];
  const viewerInvitations: Obj[] = [
    {
      id: 'inv-globex',
      email: 'mika@acme.dev',
      role: 'ADMIN',
      state: 'PENDING',
      expiresAt: ahead(60 * 24 * 6),
      tenantName: 'globex',
      invitedBy: { id: 'member-hana', user: users.hana },
    },
  ];
  const operatorTenants = [
    acme,
    northwind,
    operatorOnly('tenant-indie', 'indie-labs', 'Free', { kind: 'USER', login: 'indie' }, periodUsage(3, 4, 1, 0, 'TOTAL')),
    operatorOnly('tenant-tanaka', 'tanaka-sys', 'Max', { kind: 'ORGANIZATION', login: 'tanaka-sys' }, periodUsage(142, 232, 4, 0, 'MONTH'), 'SUSPENDED', 'abuse check'),
  ];
  return {
    tenants,
    memberships,
    viewerInvitations,
    operator: { tenants: operatorTenants, runningRuns: 11, runHoursThisMonth: 642, revenueThisMonthUsd: 320 },
  };
}

