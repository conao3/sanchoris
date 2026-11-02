import { useNavigate, useSearch } from "@tanstack/react-router"
import { ArrowRight, Ban, Check, KeyRound, ShieldCheck, TriangleAlert } from "lucide-react"
import { type FormEvent, useState } from "react"
import { Button } from "../components/Button.tsx"
import { Input, Label, Select } from "../components/Field.tsx"
import {
  ActionError,
  Dim,
  ErrorNote,
  Note,
  Skeleton,
  State,
  Sub,
  Title,
} from "../components/ui.tsx"
import {
  ApproveCliRequestDocument,
  CliPageDocument,
  type CredentialScope,
  DenyCliRequestDocument,
} from "../graphql/generated/graphql.ts"
import { t } from "../i18n/index.ts"
import { useAction } from "../lib/mutation.ts"
import { useData } from "../lib/query.ts"
import { rememberTenant } from "../lib/tenant.tsx"
import { AuthLayout } from "./CreateTenantPage.tsx"

export function cliSearch(search: Record<string, unknown>): { code?: string } {
  return typeof search.code === "string" && search.code ? { code: search.code } : {}
}

export function CliPage() {
  const { code } = useSearch({ from: "/signed-in/cli" })
  return (
    <AuthLayout>
      <Title>
        <span className="text-auth">{t("cli.title")}</span>
      </Title>
      <Sub className="mt-1.5">{t("cli.intro")}</Sub>
      {code ? <CliRequestForm code={code} /> : <CodeForm />}
    </AuthLayout>
  )
}

function CodeForm() {
  const navigate = useNavigate()
  const [code, setCode] = useState("")

  function submit(event: FormEvent) {
    event.preventDefault()
    void navigate({ to: "/cli", search: { code: code.trim() } })
  }

  return (
    <form onSubmit={submit}>
      <Label htmlFor="cli-code">{t("cli.code")}</Label>
      <Input
        id="cli-code"
        mono
        required
        autoFocus
        className="w-full"
        placeholder="WDJB-MJHT"
        value={code}
        onChange={(event) => setCode(event.target.value)}
      />
      <div className="mt-3.5">
        <Button type="submit" variant="primary" icon={ArrowRight}>
          {t("cli.continue")}
        </Button>
      </div>
    </form>
  )
}

function CliRequestForm({ code }: { code: string }) {
  const navigate = useNavigate()
  const { data, error, refetch } = useData(CliPageDocument, { userCode: code })
  const [tenantId, setTenantId] = useState<string>()
  const [scope, setScope] = useState<CredentialScope>()
  const [approve, approveState] = useAction(ApproveCliRequestDocument, {
    refetchQueries: [CliPageDocument],
  })
  const [deny, denyState] = useAction(DenyCliRequestDocument, {
    refetchQueries: [CliPageDocument],
  })

  if (error) {
    return <ErrorNote message={error.message} onRetry={refetch} />
  }
  if (!data) {
    return <Skeleton rows={4} />
  }
  const request = data.cliRequest
  if (!request) {
    return (
      <Note icon={TriangleAlert}>
        <span className="font-medium">{t("cli.unknown", { code })}</span>
        <div className="mt-2">
          <Button onPress={() => void navigate({ to: "/cli", search: {} })}>
            {t("cli.otherCode")}
          </Button>
        </div>
      </Note>
    )
  }
  const memberships = data.viewer.memberships
  const membership = memberships.find((m) => m.tenant.id === tenantId) ?? memberships[0]
  const canShare = membership?.role === "OWNER" || membership?.role === "ADMIN"
  const chosen: CredentialScope =
    (scope ?? request.requestedScope) === "ORGANIZATION" && canShare ? "ORGANIZATION" : "PERSONAL"
  const what =
    request.purpose === "CLAUDE_CREDENTIAL"
      ? t("cli.claude", { client: request.client })
      : t("cli.codex", { client: request.client })
  const busy = approveState.loading || denyState.loading

  function submit(event: FormEvent) {
    event.preventDefault()
    if (!membership || !request) {
      return
    }
    void approve({
      input: { userCode: request.userCode, tenantId: membership.tenant.id, scope: chosen },
    })
  }

  function openCredentials(slug: string) {
    rememberTenant(slug)
    window.location.assign("/settings/credentials")
  }

  return (
    <>
      <div className="mb-3 mt-2 flex items-center gap-3">
        <span className="font-mono text-h1 font-semibold tracking-wider">{request.userCode}</span>
      </div>
      <Dim className="mb-3">{what}</Dim>
      {request.state === "APPROVED" ? (
        <Note icon={ShieldCheck}>
          <span className="font-medium">{t("cli.approved")}</span>
          {membership ? (
            <div className="mt-2">
              <Button icon={KeyRound} onPress={() => openCredentials(membership.tenant.slug)}>
                {t("cli.openCredentials")}
              </Button>
            </div>
          ) : null}
        </Note>
      ) : null}
      {request.state === "DENIED" ? (
        <Note icon={Ban}>
          <span className="font-medium">{t("cli.denied")}</span>
        </Note>
      ) : null}
      {request.state === "EXPIRED" ? (
        <Note icon={TriangleAlert}>
          <span className="font-medium">{t("cli.expired")}</span>
        </Note>
      ) : null}
      {request.state === "PENDING" && memberships.length === 0 ? (
        <Note icon={TriangleAlert}>
          <span className="font-medium">{t("cli.noTenant")}</span>
          <div className="mt-2">
            <Button onPress={() => void navigate({ to: "/tenants/new" })}>
              {t("nav.createTenant")}
            </Button>
          </div>
        </Note>
      ) : null}
      {request.state === "PENDING" && membership ? (
        <form onSubmit={submit}>
          <Label htmlFor="cli-tenant">{t("cli.tenant")}</Label>
          <Select
            id="cli-tenant"
            className="w-full"
            value={membership.tenant.id}
            onChange={(event) => setTenantId(event.target.value)}
          >
            {memberships.map((m) => (
              <option key={m.id} value={m.tenant.id}>
                {m.tenant.name}
              </option>
            ))}
          </Select>
          <Label htmlFor="cli-scope">{t("cli.scope")}</Label>
          <Select
            id="cli-scope"
            className="w-full"
            value={chosen}
            onChange={(event) => setScope(event.target.value as CredentialScope)}
          >
            <option value="PERSONAL">{t("cli.personal")}</option>
            <option value="ORGANIZATION" disabled={!canShare}>
              {canShare ? t("cli.org") : `${t("cli.org")} · ${t("reason.adminOrgKey")}`}
            </option>
          </Select>
          <Dim className="mt-3">
            <State tone="warn" icon={TriangleAlert} wrap>
              {t("cli.warning")}
            </State>
          </Dim>
          <Dim className="mt-1">
            {t("cli.expires", {
              time: new Date(request.expiresAt).toLocaleTimeString([], {
                hour: "2-digit",
                minute: "2-digit",
              }),
            })}
          </Dim>
          <ActionError error={approveState.error ?? denyState.error} className="mt-2" />
          <div className="mt-3.5 flex gap-2">
            <Button type="submit" variant="primary" icon={Check} isDisabled={busy}>
              {t("cli.approve")}
            </Button>
            <Button
              icon={Ban}
              isDisabled={busy}
              onPress={() => void deny({ userCode: request.userCode })}
            >
              {t("cli.deny")}
            </Button>
          </div>
        </form>
      ) : null}
    </>
  )
}
