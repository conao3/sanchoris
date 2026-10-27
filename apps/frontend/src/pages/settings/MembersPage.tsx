import { useMutation } from '@apollo/client/react';
import { Ban, Plus, Send, ShieldCheck, TriangleAlert } from 'lucide-react';
import { type FormEvent, type RefObject, useRef, useState } from 'react';
import { Button } from '../../components/Button';
import { SettingsPage } from '../../components/SettingsTabs';
import { Dim, ErrorNote, Icon, Id, Row, Section, Skeleton, State, Sub, Th, Title } from '../../components/ui';
import {
  ChangeMemberRoleDocument,
  InviteMemberDocument,
  MembersDocument,
  type MembersQuery,
  RevokeInvitationDocument,
  type Role,
} from '../../graphql/generated/graphql';
import { type MessageKey, t } from '../../i18n';
import { ago, until } from '../../lib/format';
import { useListNav } from '../../lib/keys';
import { useData } from '../../lib/query';
import { useTenant } from '../../lib/tenant';

type Member = NonNullable<MembersQuery['tenant']>['members'][number];

const roleLabels: Record<Role, MessageKey> = {
  OWNER: 'role.owner',
  ADMIN: 'role.admin',
  MEMBER: 'role.member',
};

const fieldClass = 'h-[30px] rounded-ui border border-line2 bg-bg px-2.5 outline-none focus:border-ink';

function CredentialCell({ member, provider }: { member: Member; provider: 'CLAUDE' | 'CODEX' }) {
  const credential = member.credentials.find((item) => item.provider === provider);
  if (!credential) {
    return <span className="text-muted">—</span>;
  }
  return credential.status === 'RECONNECT' ? (
    <State tone="err" icon={TriangleAlert}>
      {credential.name}
    </State>
  ) : (
    <State tone="ok" icon={ShieldCheck}>
      {credential.name}
    </State>
  );
}

function InviteForm({ tenantId, emailRef, onDone }: { tenantId: string; emailRef: RefObject<HTMLInputElement | null>; onDone: () => void }) {
  const [email, setEmail] = useState('');
  const [role, setRole] = useState<Role>('MEMBER');
  const [invite, { loading, error }] = useMutation(InviteMemberDocument, { refetchQueries: [MembersDocument] });

  function submit(event: FormEvent) {
    event.preventDefault();
    void invite({ variables: { input: { tenantId, email, role } } }).then(() => {
      setEmail('');
      onDone();
    });
  }

  return (
    <form onSubmit={submit} className="mb-4 flex items-center gap-2 border-b border-line pb-4">
      <span className="text-muted">{t('members.inviteBy')}</span>
      <input
        ref={emailRef}
        type="email"
        required
        placeholder="ren@acme.dev"
        aria-label={t('col.email')}
        className={`${fieldClass} w-64 font-mono text-sm`}
        value={email}
        onChange={(event) => setEmail(event.target.value)}
      />
      <select aria-label={t('col.role')} className={fieldClass} value={role} onChange={(event) => setRole(event.target.value as Role)}>
        <option value="MEMBER">{t('role.member')}</option>
        <option value="ADMIN">{t('role.admin')}</option>
      </select>
      <Button type="submit" variant="primary" icon={Send} isDisabled={loading}>
        {t('members.sendInvitation')}
      </Button>
      {error ? <span className="text-err">{error.message}</span> : null}
    </form>
  );
}

export function MembersPage() {
  const { slug } = useTenant();
  const { data, error, refetch } = useData(MembersDocument, { tenant: slug });
  const [changeRole] = useMutation(ChangeMemberRoleDocument);
  const [revoke] = useMutation(RevokeInvitationDocument, { refetchQueries: [MembersDocument] });
  const [inviting, setInviting] = useState(false);
  const emailRef = useRef<HTMLInputElement>(null);
  const tenant = data?.tenant;
  const members = tenant?.members ?? [];
  const nav = useListNav(members.length, () => undefined, {
    i: () => {
      setInviting(true);
      window.setTimeout(() => emailRef.current?.focus());
    },
  });
  const viewerRole = tenant?.viewerMember.role;
  const canManage = viewerRole === 'OWNER' || viewerRole === 'ADMIN';
  const cols = '1fr 100px 130px 130px 110px 110px';
  const inviteCols = '1fr 100px 160px 140px 100px';

  return (
    <SettingsPage
      tab="members"
      actions={
        canManage ? (
          <Button variant="primary" icon={Plus} kbd="i" onPress={() => setInviting(!inviting)}>
            {t('members.invite')}
          </Button>
        ) : null
      }
    >
      <Title>{t('settings.members')}</Title>
      <Sub>{tenant ? t('members.intro', { n: tenant.usage.seatsUsed, total: tenant.plan.memberLimit, tenant: tenant.name }) : ' '}</Sub>
      {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
      {!data && !error ? <Skeleton rows={6} /> : null}
      {tenant && inviting ? <InviteForm tenantId={tenant.id} emailRef={emailRef} onDone={() => setInviting(false)} /> : null}
      {tenant ? (
        <>
          <Th cols={cols}>
            <span>{t('col.member')}</span>
            <span>{t('col.role')}</span>
            <span>{t('provider.claude')}</span>
            <span>{t('provider.codex')}</span>
            <span className="text-right">{t('col.tasksStarted')}</span>
            <span className="text-right">{t('col.lastActive')}</span>
          </Th>
          {members.map((member, index) => (
            <Row key={member.id} cols={cols} selected={nav.selected === index} rowRef={nav.rowRef(index)} onClick={() => nav.select(index)}>
              <div>
                <span className="font-medium">{member.user.name}</span> <Id>{member.user.githubLogin ?? t('members.githubNotLinked')}</Id>
              </div>
              {canManage && member.role !== 'OWNER' && (viewerRole === 'OWNER' || member.role !== 'ADMIN') ? (
                <select
                  aria-label={t('col.role')}
                  className="h-6 rounded-ui border border-line2 bg-bg px-1 text-sm"
                  value={member.role}
                  onClick={(event) => event.stopPropagation()}
                  onChange={(event) => void changeRole({ variables: { memberId: member.id, role: event.target.value as Role } })}
                >
                  {viewerRole === 'OWNER' ? <option value="ADMIN">{t('role.admin')}</option> : null}
                  <option value="MEMBER">{t('role.member')}</option>
                </select>
              ) : (
                <span>
                  {member.role === 'MEMBER' ? null : <Icon icon={ShieldCheck} />} {t(roleLabels[member.role])}
                </span>
              )}
              <CredentialCell member={member} provider="CLAUDE" />
              <CredentialCell member={member} provider="CODEX" />
              <span className="text-right font-mono text-sm">{member.tasksStarted}</span>
              <span className="text-right text-muted">{member.lastActiveAt ? ago(member.lastActiveAt) : t('credentials.never')}</span>
            </Row>
          ))}
          <Section count={tenant.invitations.length}>{t('members.pending')}</Section>
          {tenant.invitations.length === 0 ? <Dim>{t('members.noPending')}</Dim> : null}
          {tenant.invitations.length > 0 ? (
            <>
              <Th cols={inviteCols}>
                <span>{t('col.invited')}</span>
                <span>{t('col.role')}</span>
                <span>{t('col.invitedBy')}</span>
                <span>{t('col.expires')}</span>
                <span />
              </Th>
              {tenant.invitations.map((invitation) => (
                <Row key={invitation.id} cols={inviteCols}>
                  <span className="font-mono text-sm">{invitation.email}</span>
                  <span>{t(roleLabels[invitation.role])}</span>
                  <span>{invitation.invitedBy.user.name}</span>
                  <span className="text-muted">{until(invitation.expiresAt)}</span>
                  <span className="justify-self-end">
                    {canManage ? (
                      <Button small icon={Ban} onPress={() => void revoke({ variables: { invitationId: invitation.id } })}>
                        {t('members.revoke')}
                      </Button>
                    ) : null}
                  </span>
                </Row>
              ))}
            </>
          ) : null}
          <Dim className="mt-3.5">{t('members.footer')}</Dim>
        </>
      ) : null}
    </SettingsPage>
  );
}
