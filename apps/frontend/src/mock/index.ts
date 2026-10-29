import { SchemaLink } from '@apollo/client/link/schema';
import { addMocksToSchema } from '@graphql-tools/mock';
import { makeExecutableSchema } from '@graphql-tools/schema';
import typeDefs from '../graphql/schema.graphql?raw';
import { buildFixtures, buildSetupTenant, nextId, plans, triggerTemplates, viewerUser } from './fixtures';

type Obj = Record<string, any>;

function createResolvers() {
  const fixtures = buildFixtures();
  const { tenants, memberships, viewerInvitations, operator } = fixtures;
  const viewer = { user: viewerUser, memberships, invitations: viewerInvitations, isOperator: true, newTenantPlans: plans };

  const tenantById = (id: string): Obj => {
    const tenant = tenants.find((item) => item.id === id) ?? operator.tenants.find((item) => item.id === id);
    if (!tenant) {
      throw new Error(`Tenant ${id} was not found`);
    }
    return tenant;
  };
  const allTasks = () => tenants.flatMap((tenant) => tenant.taskList as Obj[]);
  const taskById = (id: string): Obj => {
    const task = allTasks().find((item) => item.id === id);
    if (!task) {
      throw new Error(`Task ${id} was not found`);
    }
    return task;
  };
  const touch = (task: Obj) => {
    task.updatedAt = new Date().toISOString();
  };
  const appendLog = (run: Obj, kind: string, text: string) => {
    run.lines.push({ sequence: run.lines.length + 1, at: new Date().toISOString(), kind, text });
  };
  const viewerMemberOf = (tenant: Obj): Obj => tenant.viewerMember;

  return {
    Query: {
      viewer: () => viewer,
      plans: () => plans,
      tenant: (_: unknown, args: { slug: string }) => tenants.find((tenant) => tenant.slug === args.slug) ?? null,
      operator: () => operator,
      triggerTemplates: () => triggerTemplates,
    },
    Mutation: {
      createTenant: (_: unknown, { input }: Obj) => {
        const plan = plans.find((item) => item.id === input.planId);
        if (!plan) {
          throw new Error('Unknown plan');
        }
        const slug = String(input.name).toLowerCase().replace(/[^a-z0-9-]+/g, '-');
        const tenant = buildSetupTenant({ slug, name: input.name, planName: plan.name, role: 'OWNER', installed: false });
        tenants.push(tenant);
        memberships.push(tenant.viewerMember);
        if (plan.requiresPayment) {
          tenant.state = 'SUSPENDED';
          tenant.suspendedReason = 'payment_pending';
          return { tenant, checkoutUrl: `https://checkout.stripe.com/c/pay/cs_test_${encodeURIComponent(input.name)}` };
        }
        return { tenant, checkoutUrl: null };
      },
      acceptInvitation: (_: unknown, { invitationId }: Obj) => {
        const index = viewerInvitations.findIndex((invitation) => invitation.id === invitationId);
        if (index < 0) {
          throw new Error('The invitation was not found');
        }
        const [invitation] = viewerInvitations.splice(index, 1);
        const tenant = buildSetupTenant({ slug: invitation.tenantName, name: invitation.tenantName, planName: 'Pro', role: invitation.role, installed: true });
        tenants.push(tenant);
        memberships.push(tenant.viewerMember);
        return tenant.viewerMember;
      },
      connectGitHub: () => ({ url: '/auth/github' }),
      unlinkGitHub: () => {
        viewerUser.githubLogin = null;
        return viewer;
      },
      connectSlack: () => ({ url: 'https://slack.com/oauth/v2/authorize?client_id=mock' }),
      inviteMember: (_: unknown, { input }: Obj) => {
        const tenant = tenantById(input.tenantId);
        if (tenant.usage.seatsUsed >= tenant.plan.memberLimit) {
          throw new Error(`The plan allows ${tenant.plan.memberLimit} members, including pending invitations.`);
        }
        const invitation = {
          id: nextId('inv'),
          email: input.email,
          role: input.role,
          state: 'PENDING',
          invitedBy: viewerMemberOf(tenant),
          expiresAt: new Date(Date.now() + 7 * 86_400_000).toISOString(),
          tenantName: tenant.name,
        };
        tenant.invitationList.unshift(invitation);
        tenant.usage.seatsUsed += 1;
        return invitation;
      },
      revokeInvitation: (_: unknown, { invitationId }: Obj) => {
        const tenant = tenants.find((item) => item.invitationList.some((invitation: Obj) => invitation.id === invitationId));
        const invitation = tenant?.invitationList.find((item: Obj) => item.id === invitationId);
        if (!tenant || !invitation) {
          throw new Error('The invitation was not found');
        }
        invitation.state = 'REVOKED';
        tenant.usage.seatsUsed -= 1;
        return invitation;
      },
      changeMemberRole: (_: unknown, { memberId, role }: Obj) => {
        const member = tenants.flatMap((tenant) => tenant.memberList as Obj[]).find((item) => item.id === memberId);
        if (!member) {
          throw new Error('The member was not found');
        }
        member.role = role;
        return member;
      },
      removeMember: (_: unknown, { memberId }: Obj) => {
        for (const tenant of tenants) {
          const index = tenant.memberList.findIndex((member: Obj) => member.id === memberId);
          if (index >= 0) {
            tenant.memberList.splice(index, 1);
          }
        }
        return memberId;
      },
      resumeTask: (_: unknown, { input }: Obj) => {
        const task = taskById(input.taskId);
        if (task.status !== 'NEEDS_YOU') {
          throw new Error('Only a task that needs you can be resumed.');
        }
        const run = task.runs[task.runs.length - 1];
        if (run) {
          appendLog(run, 'USER', input.message);
          appendLog(run, 'INSTRUCTION', `${input.message}\nThe required skill runs again after your change.`);
          run.status = 'RUNNING';
        }
        task.status = 'RUNNING';
        task.needsYouReason = null;
        task.error = null;
        task.cause = null;
        touch(task);
        return task;
      },
      snoozeTask: (_: unknown, { taskId, until }: Obj) => {
        const task = taskById(taskId);
        task.snoozedUntil = until;
        touch(task);
        return task;
      },
      resolveApproval: (_: unknown, { taskId, approved }: Obj) => {
        const task = taskById(taskId);
        if (!task.approval) {
          throw new Error('the task has no approval waiting');
        }
        task.approval = null;
        task.status = 'WAITING';
        task.waitReason = 'SKILL';
        task.needsYouReason = null;
        task.error = null;
        task.cause = approved ? null : 'Mika rejected the request';
        touch(task);
        return task;
      },
      stopTask: (_: unknown, { taskId }: Obj) => {
        const task = taskById(taskId);
        task.status = 'ABORTED';
        task.endedAt = new Date().toISOString();
        for (const run of task.runs) {
          if (run.status !== 'COMPLETED' && run.status !== 'FAILED') {
            run.status = 'COMPLETED';
          }
        }
        touch(task);
        return task;
      },
      stopRun: (_: unknown, { runId }: Obj) => {
        const run = allTasks()
          .flatMap((task) => task.runs as Obj[])
          .find((item) => item.id === runId);
        if (!run) {
          throw new Error('The run was not found');
        }
        run.status = 'COMPLETED';
        appendLog(run, 'EVENT', 'session stopped by a member');
        return run;
      },
      sendChatMessage: (_: unknown, { input }: Obj) => {
        const tenant = tenantById(input.tenantId);
        const message = { id: nextId('msg'), authorKind: 'MEMBER', member: viewerMemberOf(tenant), text: input.text, at: new Date().toISOString(), issues: [] };
        if (input.taskId) {
          const task = taskById(input.taskId);
          task.messages.push(message);
          const run = task.runs[task.runs.length - 1];
          if (run) {
            appendLog(run, 'USER', input.text);
          }
          if (task.status === 'WAITING' && task.waitReason === 'CHAT') {
            task.status = 'RUNNING';
            task.waitReason = null;
          }
          touch(task);
          return { task, message };
        }
        const trigger = tenant.triggerList.find((item: Obj) => item.match.source === 'WEB' && item.match.chatScope === input.scope && item.state === 'ENABLED');
        if (!trigger) {
          throw new Error(`No enabled chat trigger has the scope ${input.scope}.`);
        }
        const stamp = new Date().toISOString();
        const task: Obj = {
          id: nextId('task-chat'),
          name: nextId('web-thread'),
          title: input.text.length > 48 ? `${input.text.slice(0, 48)}…` : input.text,
          status: 'RUNNING',
          waitReason: null,
          pendingReason: null,
          needsYouReason: null,
          approval: null,
          resumeAt: null,
          autoResume: true,
          snoozedUntil: null,
          nudges: 0,
          maxNudges: trigger.maxNudges,
          error: null,
          cause: null,
          createdAt: stamp,
          startedAt: stamp,
          endedAt: null,
          updatedAt: stamp,
          trigger,
          triggerEvent: null,
          requester: { kind: 'MEMBER', member: viewerMemberOf(tenant), task: null, viaChat: true },
          session: trigger.session,
          configRevision: 41,
          issue: null,
          channel: { kind: 'WEB', threadId: nextId('web-thread'), scope: input.scope, slackChannel: null },
          parent: null,
          children: () => [],
          blockedBy: [],
          filedTasks: () => [],
          requiredSkills: () => [],
          skillRuns: () => [],
          runs: [],
          run: () => null,
          outputs: [],
          messages: [message],
          limits: trigger.limits,
          turns: 0,
          costUsd: 0,
        };
        tenant.taskList.push(task);
        return { task, message };
      },
      setTriggerState: (_: unknown, { triggerId, state }: Obj) => {
        const trigger = tenants.flatMap((tenant) => tenant.triggerList as Obj[]).find((item) => item.id === triggerId);
        if (!trigger) {
          throw new Error('The trigger was not found');
        }
        trigger.state = state;
        return trigger;
      },
      proposeTriggerFromTemplate: (_: unknown, { input }: Obj) => {
        const tenant = tenantById(input.tenantId);
        const config = tenant.configs().find((item: Obj) => item.id === input.configId);
        const template = triggerTemplates.find((item) => item.id === input.templateId);
        if (!config || !template) {
          throw new Error('The config or the template was not found');
        }
        const number = 8 + tenant.changeRequestList.length;
        const change = {
          id: nextId('cr'),
          config,
          number,
          title: `Add the ${template.name} trigger for ${input.repository}`,
          url: `https://github.com/${config.repository}/pull/${number}`,
          state: 'OPEN',
          changes: [{ kind: 'TRIGGER', name: template.name }],
          validation: { valid: true, errors: [] },
          openedFromTask: null,
          openedBy: viewerMemberOf(tenant),
          openedAt: new Date().toISOString(),
        };
        tenant.changeRequestList.push(change);
        return change;
      },
      addConfig: (_: unknown, { input }: Obj) => {
        if (!String(input.root).endsWith('/')) {
          return { config: null, errors: [{ path: input.root, line: null, column: null, message: 'The root must be a directory and end with /.' }] };
        }
        return {
          config: null,
          errors: [{ path: `${input.root}config.yaml`, line: 4, column: 7, message: `Triggers.labeled-issue.EventPattern.detail.repository must name a repository in ${input.repository}.` }],
        };
      },
      syncConfig: (_: unknown, { configId }: Obj) => {
        const config = tenants.flatMap((tenant) => tenant.configs() as Obj[]).find((item) => item.id === configId);
        if (!config) {
          throw new Error('The config was not found');
        }
        config.syncedAt = new Date().toISOString();
        config.syncError = null;
        return config;
      },
      addCredential: (_: unknown, { input }: Obj) => {
        const tenant = tenantById(input.tenantId);
        const credential = {
          id: nextId('cred'),
          name: input.name,
          provider: input.provider === 'OPENAI' ? 'CODEX' : 'CLAUDE',
          kind: input.provider === 'BEDROCK' ? 'BEDROCK' : 'API_KEY',
          subscriptionPlan: null,
          account: `${String(input.key).slice(0, 7)}…${String(input.key).slice(-4)}`,
          scope: input.scope,
          owner: input.scope === 'PERSONAL' ? viewerMemberOf(tenant) : null,
          status: 'CONNECTED',
          rejectedAt: null,
          lastUsedAt: null,
          usage: null,
        };
        tenant.credentialList.push(credential);
        return credential;
      },
      removeCredential: (_: unknown, { credentialId }: Obj) => {
        for (const tenant of tenants) {
          const index = tenant.credentialList.findIndex((credential: Obj) => credential.id === credentialId);
          if (index >= 0) {
            tenant.credentialList.splice(index, 1);
          }
        }
        return credentialId;
      },
      changePlan: (_: unknown, { tenantId, planId }: Obj) => {
        const tenant = tenantById(tenantId);
        const plan = plans.find((item) => item.id === planId);
        if (!plan) {
          throw new Error('Unknown plan');
        }
        if (plan.priceUsd > tenant.plan.priceUsd) {
          return { tenant, checkoutUrl: `https://checkout.stripe.com/c/pay/cs_test_${plan.name.toLowerCase()}` };
        }
        tenant.plan = plan;
        return { tenant, checkoutUrl: null };
      },
      openBillingPortal: () => ({ url: 'https://billing.stripe.com/p/session/test_mock' }),
      exportAuditLog: (_: unknown, { tenantId }: Obj) => {
        const tenant = tenantById(tenantId);
        const lines = (tenant.auditList as Obj[]).map((event) => JSON.stringify({ at: event.at, actor: event.actorName, action: event.action, target: event.target })).join('\n');
        return { url: URL.createObjectURL(new Blob([`${lines}\n`], { type: 'application/x-ndjson' })) };
      },
      suspendTenant: (_: unknown, { tenantId, reason }: Obj) => {
        const tenant = tenantById(tenantId);
        tenant.state = 'SUSPENDED';
        tenant.suspendedReason = reason;
        return tenant;
      },
      unsuspendTenant: (_: unknown, { tenantId }: Obj) => {
        const tenant = tenantById(tenantId);
        tenant.state = 'ACTIVE';
        tenant.suspendedReason = null;
        return tenant;
      },
    },
  };
}

export function createMockLink(): SchemaLink {
  const schema = makeExecutableSchema({ typeDefs });
  const resolvers = createResolvers();
  const mocked = addMocksToSchema<typeof resolvers>({
    schema,
    mocks: {
      DateTime: () => new Date().toISOString(),
    },
    resolvers,
  });
  return new SchemaLink({ schema: mocked });
}
