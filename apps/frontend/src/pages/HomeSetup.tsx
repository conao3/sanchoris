import { useNavigate } from "@tanstack/react-router"
import { CircleCheck, CircleDot, Clock } from "lucide-react"
import type { ReactNode } from "react"
import { ApiKeyForm, SubscriptionCommands } from "../components/ApiKeyForm.tsx"
import { Cols } from "../components/Shell.tsx"
import { Chip, Dim, Lk, Row, Section, State, Sub, Table, Th, Title } from "../components/ui.tsx"
import type { HomeQuery } from "../graphql/generated/graphql.ts"
import { type MessageKey, t, tx } from "../i18n/index.ts"
import { useKeys } from "../lib/keys.ts"
import { useTenant } from "../lib/tenant.tsx"

type Tenant = NonNullable<HomeQuery["tenant"]>

type Step = {
  key: string
  title: string
  detail: ReactNode
  done: boolean
  optional?: boolean
  where: MessageKey
  to: string
}

export function HomeSetup({ tenant }: { tenant: Tenant }) {
  const navigate = useNavigate()
  const { viewer } = useTenant()
  const { setup } = tenant
  const github = tenant.integrations.find(
    (integration) => integration.kind === "GITHUB_INSTALLATION",
  )
  const config = tenant.configs[0]

  const steps: Step[] = [
    {
      key: "github",
      title: t("setup.step1"),
      detail:
        setup.githubLinked && setup.appInstalled
          ? tx("setup.step1Done", {
              login: <Chip>{viewer.user.githubLogin}</Chip>,
              account: <Chip>{github?.account}</Chip>,
              n: github?.repositories.length ?? 0,
            })
          : t("setup.step1Todo"),
      done: setup.githubLinked && setup.appInstalled,
      where: "setup.whereIntegrations",
      to: "/settings",
    },
    {
      key: "config",
      title: t("setup.step2"),
      detail: tx("setup.step2Detail", {
        repo: <Chip>{config ? config.repository.split("/").pop() : "sanchoris-config"}</Chip>,
      }),
      done: setup.configAdded,
      where: "setup.whereConfig",
      to: "/settings/config",
    },
    {
      key: "credential",
      title: t("setup.step3"),
      detail: t("setup.step3Detail"),
      done: setup.credentialConnected,
      where: "setup.whereCredentials",
      to: "/settings/credentials",
    },
    {
      key: "trigger",
      title: t("setup.step4"),
      detail: t("setup.step4Detail"),
      done: setup.triggerEnabled,
      where: "nav.triggers",
      to: "/triggers",
    },
    {
      key: "label",
      title: t("setup.step5"),
      detail: t("setup.step5Detail"),
      done: setup.firstTaskStarted,
      where: "nav.tasks",
      to: "/tasks",
    },
    {
      key: "project",
      title: t("setup.project"),
      detail: t("setup.projectDetail"),
      done: setup.projectCreated,
      optional: true,
      where: "setup.whereConfig",
      to: "/settings/config",
    },
    {
      key: "slack",
      title: t("setup.slack"),
      detail: t("setup.slackDetail"),
      done: setup.slackConnected,
      optional: true,
      where: "setup.whereIntegrations",
      to: "/settings",
    },
  ]
  const current = steps.find((step) => !step.done && !step.optional)

  useKeys({
    Enter: () => {
      if (current) {
        void navigate({ to: current.to })
      }
    },
  })

  const cols = "90px 1fr 190px"
  const main = (
    <>
      <Title>{t("nav.home")}</Title>
      <Sub>{t("setup.intro", { tenant: tenant.name })}</Sub>
      <Table>
        <Th cols={cols}>
          <span>{t("col.step")}</span>
          <span />
          <span />
        </Th>
        {steps.map((step) => {
          const state = step.done ? (
            <State tone="ok" icon={CircleCheck}>
              {t("setup.done")}
            </State>
          ) : step === current ? (
            <State tone="run" icon={CircleDot}>
              {t("setup.now")}
            </State>
          ) : (
            <State tone="wait" icon={Clock}>
              {step.optional ? t("setup.optional") : t("setup.next")}
            </State>
          )
          return (
            <Row key={step.key} cols={cols} tall selected={step === current}>
              {state}
              <div>
                <span className="font-medium">{step.title}</span>
                <Dim>{step.detail}</Dim>
              </div>
              <span className="text-muted">
                <Lk to={step.to}>{t(step.where)}</Lk>
              </span>
            </Row>
          )
        })}
      </Table>
      <Section>{t("setup.connectSubscription")}</Section>
      <Dim className="mb-2">{t("setup.connectSubscriptionDetail")}</Dim>
      <SubscriptionCommands tenantName={tenant.name} />
    </>
  )

  const aside = (
    <>
      <Section first>{t("setup.orApiKey")}</Section>
      <Dim>{t("setup.apiKeyProviders")}</Dim>
      <ApiKeyForm tenantId={tenant.id} />
      <Section>{t("setup.usedBySession")}</Section>
      <Dim>{t("setup.usedBySessionDetail")}</Dim>
    </>
  )

  return <Cols main={main} aside={aside} />
}
