import { Ban, Plus, Send, ShieldCheck, TriangleAlert } from "lucide-react"
import { type FormEvent, type RefObject, useRef, useState } from "react"
import { Button } from "../../components/Button.tsx"
import { SettingsPage } from "../../components/SettingsTabs.tsx"
import {
  ActionError,
  Dim,
  ErrorNote,
  Icon,
  Id,
  Row,
  Section,
  Skeleton,
  State,
  Sub,
  Table,
  Th,
  Title,
} from "../../components/ui.tsx"
import {
  ChangeMemberRoleDocument,
  InviteMemberDocument,
  MembersDocument,
  type MembersQuery,
  RevokeInvitationDocument,
  type Role,
} from "../../graphql/generated/graphql.ts"
import { type MessageKey, t } from "../../i18n/index.ts"
import { ago, until } from "../../lib/format.ts"
import { listKeys, useListNav } from "../../lib/keys.ts"
import { useAction } from "../../lib/mutation.ts"
import { useData } from "../../lib/query.ts"
import { useRole, useTenant } from "../../lib/tenant.tsx"

type Member = NonNullable<MembersQuery["tenant"]>["members"][number]

const roleLabels: Record<Role, MessageKey> = {
  OWNER: "role.owner",
  ADMIN: "role.admin",
  MEMBER: "role.member",
}

const fieldClass =
  "h-[30px] rounded-ui border border-line2 bg-bg px-2.5 outline-none focus:border-ink"

function CredentialCell({ member, provider }: { member: Member; provider: "CLAUDE" | "CODEX" }) {
  const credential = member.credentials.find((item) => item.provider === provider)
  if (!credential) {
    return <span className="text-muted">—</span>
  }
  return credential.status === "RECONNECT" ? (
    <State tone="err" icon={TriangleAlert}>
      {credential.name}
    </State>
  ) : (
    <State tone="ok" icon={ShieldCheck}>
      {credential.name}
    </State>
  )
}

function InviteForm({
  tenantId,
  emailRef,
  onDone,
}: {
  tenantId: string
  emailRef: RefObject<HTMLInputElement | null>
  onDone: () => void
}) {
  const [email, setEmail] = useState("")
  const [role, setRole] = useState<Role>("MEMBER")
  const { isOwner } = useRole()
  const [invite, { loading, error }] = useAction(InviteMemberDocument, {
    refetchQueries: [MembersDocument],
  })

  function submit(event: FormEvent) {
    event.preventDefault()
    void invite({ input: { tenantId, email, role } }).then((done) => {
      if (!done) {
        return
      }
      setEmail("")
      onDone()
    })
  }

  return (
    <form
      onSubmit={submit}
      className="mb-4 flex flex-wrap items-center gap-2 border-b border-line pb-4"
    >
      <span className="text-muted">{t("members.inviteBy")}</span>
      <input
        ref={emailRef}
        type="email"
        required
        placeholder="ren@acme.dev"
        aria-label={t("col.email")}
        className={`${fieldClass} w-64 font-mono text-sm`}
        value={email}
        onChange={(event) => setEmail(event.target.value)}
      />
      {isOwner ? (
        <select
          aria-label={t("col.role")}
          className={fieldClass}
          value={role}
          onChange={(event) => setRole(event.target.value as Role)}
        >
          <option value="MEMBER">{t("role.member")}</option>
          <option value="ADMIN">{t("role.admin")}</option>
        </select>
      ) : null}
      <Button type="submit" variant="primary" icon={Send} isDisabled={loading}>
        {t("members.sendInvitation")}
      </Button>
      <ActionError error={error} />
    </form>
  )
}

export function MembersPage() {
  const { slug } = useTenant()
  const { data, error, refetch } = useData(MembersDocument, { tenant: slug })
  const [changeRole, roleState] = useAction(ChangeMemberRoleDocument)
  const [revoke, revokeState] = useAction(RevokeInvitationDocument, {
    refetchQueries: [MembersDocument],
  })
  const [inviting, setInviting] = useState(false)
  const emailRef = useRef<HTMLInputElement>(null)
  const tenant = data?.tenant
  const members = tenant?.members ?? []
  const viewerRole = tenant?.viewerMember.role
  const canManage = viewerRole === "OWNER" || viewerRole === "ADMIN"
  const nav = useListNav(
    members.length,
    () => undefined,
    canManage
      ? {
          i: () => {
            setInviting(true)
            window.setTimeout(() => emailRef.current?.focus())
          },
        }
      : {},
  )
  const cols = "1fr 100px 130px 130px 110px 110px"
  const inviteCols = "1fr 100px 160px 140px 100px"

  return (
    <SettingsPage
      tab="members"
      keys={[
        ["g s", "nav.settings"],
        ...listKeys(members.length),
        ...(canManage ? [["i", "members.invite"] as [string, MessageKey]] : []),
      ]}
      actions={
        canManage ? (
          <Button variant="primary" icon={Plus} kbd="i" onPress={() => setInviting(!inviting)}>
            {t("members.invite")}
          </Button>
        ) : null
      }
    >
      <Title>{t("settings.members")}</Title>
      <Sub>
        {tenant
          ? tenant.plan.memberLimit == null
            ? t("members.introNoLimit", { n: tenant.usage.seatsUsed, tenant: tenant.name })
            : t("members.intro", {
                n: tenant.usage.seatsUsed,
                total: tenant.plan.memberLimit,
                tenant: tenant.name,
              })
          : " "}
      </Sub>
      {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
      {!data && !error ? <Skeleton rows={6} /> : null}
      {tenant && inviting ? (
        <InviteForm tenantId={tenant.id} emailRef={emailRef} onDone={() => setInviting(false)} />
      ) : null}
      {tenant ? (
        <>
          <Table>
            <Th cols={cols}>
              <span>{t("col.member")}</span>
              <span>{t("col.role")}</span>
              <span>{t("provider.claude")}</span>
              <span>{t("provider.codex")}</span>
              <span className="text-right">{t("col.tasksStarted")}</span>
              <span className="text-right">{t("col.lastActive")}</span>
            </Th>
            {members.map((member, index) => (
              <Row
                key={member.id}
                cols={cols}
                selected={nav.selected === index}
                rowRef={nav.rowRef(index)}
                onClick={nav.pick(index)}
              >
                <div>
                  <span className="font-medium">{member.user.name}</span>{" "}
                  <Id>{member.user.githubLogin ?? t("members.githubNotLinked")}</Id>
                </div>
                {viewerRole === "OWNER" && member.role !== "OWNER" ? (
                  <select
                    aria-label={t("col.role")}
                    className="h-7 rounded-ui border border-line2 bg-bg px-1 text-sm"
                    value={member.role}
                    onClick={(event) => event.stopPropagation()}
                    disabled={roleState.loading}
                    onChange={(event) =>
                      void changeRole({ memberId: member.id, role: event.target.value as Role })
                    }
                  >
                    <option value="ADMIN">{t("role.admin")}</option>
                    <option value="MEMBER">{t("role.member")}</option>
                  </select>
                ) : (
                  <span>
                    {member.role === "MEMBER" ? null : <Icon icon={ShieldCheck} />}{" "}
                    {t(roleLabels[member.role])}
                  </span>
                )}
                <CredentialCell member={member} provider="CLAUDE" />
                <CredentialCell member={member} provider="CODEX" />
                <span className="text-right font-mono text-sm">{member.tasksStarted}</span>
                <span className="text-right text-muted">
                  {member.lastActiveAt ? ago(member.lastActiveAt) : t("credentials.never")}
                </span>
              </Row>
            ))}
          </Table>
          <ActionError error={roleState.error} className="mt-1.5" />
          <Section count={tenant.invitations.length}>{t("members.pending")}</Section>
          {tenant.invitations.length === 0 ? <Dim>{t("members.noPending")}</Dim> : null}
          {tenant.invitations.length > 0 ? (
            <Table>
              <Th cols={inviteCols}>
                <span>{t("col.invited")}</span>
                <span>{t("col.role")}</span>
                <span>{t("col.invitedBy")}</span>
                <span>{t("col.expires")}</span>
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
                      <Button
                        small
                        icon={Ban}
                        onPress={() => void revoke({ invitationId: invitation.id })}
                        isDisabled={revokeState.loading}
                      >
                        {t("members.revoke")}
                      </Button>
                    ) : null}
                  </span>
                </Row>
              ))}
            </Table>
          ) : null}
          <ActionError error={revokeState.error} className="mt-1.5" />
          <Dim className="mt-3.5">{t("members.footer")}</Dim>
        </>
      ) : null}
    </SettingsPage>
  )
}
