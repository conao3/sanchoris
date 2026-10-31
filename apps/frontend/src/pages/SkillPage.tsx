import { useNavigate, useParams } from "@tanstack/react-router"
import { ExternalLink, GitPullRequest } from "lucide-react"
import { useState } from "react"
import { Button } from "../components/Button.tsx"
import { FlowDiagram } from "../components/FlowDiagram.tsx"
import { Cols, Page } from "../components/Shell.tsx"
import { SkillRunState } from "../components/status.tsx"
import {
  Code,
  Dim,
  Empty,
  ErrorNote,
  Id,
  Kv,
  Lk,
  Row,
  Section,
  Skeleton,
  State,
  Sub,
  Table,
  Th,
  Title,
} from "../components/ui.tsx"
import { SkillDocument } from "../graphql/generated/graphql.ts"
import { type MessageKey, t } from "../i18n/index.ts"
import { between, duration, durationLong, issueLabel, percent, when } from "../lib/format.ts"
import { useKeys } from "../lib/keys.ts"
import { useData } from "../lib/query.ts"
import { useTenant } from "../lib/tenant.tsx"

export function SkillPage() {
  const { skill: name } = useParams({ from: "/app/skills/$skill" })
  const { slug } = useTenant()
  const navigate = useNavigate()
  const { data, error, refetch } = useData(SkillDocument, { tenant: slug, name })
  const skill = data?.tenant?.skill
  const busiest = skill?.states.find((state) => state.activity.running > 0)?.name
  const [selected, setSelected] = useState<string>()

  useKeys({
    e: () => {
      if (skill?.editUrl) {
        window.open(skill.editUrl, "_blank", "noreferrer")
      }
    },
    Escape: () => void navigate({ to: "/skills" }),
  })

  const cols = "96px 1fr 100px 80px 70px 70px"
  let body
  if (error) {
    body = (
      <div className="px-4 py-6 md:px-6">
        <ErrorNote message={error.message} onRetry={refetch} />
      </div>
    )
  } else if (!data) {
    body = (
      <div className="px-4 py-6 md:px-6">
        <Skeleton rows={6} />
      </div>
    )
  } else if (!skill) {
    body = (
      <div className="px-4 py-6 md:px-6">
        <Empty>{t("skill.notFound", { name })}</Empty>
      </div>
    )
  } else {
    const { stats } = skill
    const main = (
      <>
        <Title>{skill.name}</Title>
        <Sub>{skill.comment ?? skill.description}</Sub>
        {skill.kind === "RELIABLE" && skill.states.length > 0 ? (
          <FlowDiagram
            states={skill.states}
            startAt={skill.startAt}
            selected={selected ?? busiest}
            onSelect={(state) => setSelected(state === selected ? undefined : state)}
          />
        ) : (
          <Code>{t("skill.plainNote")}</Code>
        )}
        <Section count={t("skill.recentRunsOf", { n: stats.runs, days: stats.days })}>
          {t("skill.recentRuns")}
        </Section>
        {skill.recentRuns.length === 0 ? <Dim>{t("skill.noRuns")}</Dim> : null}
        {skill.recentRuns.length > 0 ? (
          <Table>
            <Th cols={cols}>
              <span>{t("col.status")}</span>
              <span>{t("col.task")}</span>
              <span>{t("col.atState")}</span>
              <span className="text-right">{t("col.fixRounds")}</span>
              <span>{t("col.started")}</span>
              <span className="text-right">{t("col.duration")}</span>
            </Th>
            {skill.recentRuns.map((run) => (
              <Row
                key={run.id}
                cols={cols}
                selected={!!selected && run.currentState === selected}
                onClick={() => void navigate({ to: "/tasks/$task", params: { task: run.task.id } })}
              >
                <SkillRunState status={run.status} />
                <div>
                  <span className="font-medium">{run.task.title}</span>{" "}
                  {run.task.issue ? (
                    <Id>{issueLabel(run.task.issue.repository, run.task.issue.number)}</Id>
                  ) : null}
                </div>
                <span
                  className={
                    run.status === "SUCCEEDED"
                      ? "font-mono text-sm text-muted"
                      : "font-mono text-sm"
                  }
                >
                  {run.currentState}
                </span>
                <span className="text-right font-mono text-sm">{run.handoffs}</span>
                <span className="font-mono text-sm">{when(run.startedAt)}</span>
                <span className="text-right font-mono text-sm">
                  {run.endedAt ? duration(between(run.startedAt, run.endedAt)) : "—"}
                </span>
              </Row>
            ))}
          </Table>
        ) : null}
      </>
    )
    const aside = (
      <>
        <Section first>{t("skill.skill")}</Section>
        <Kv
          items={[
            [
              t("skill.kind"),
              skill.kind === "RELIABLE" ? t("skill.reliableYaml") : t("skills.plain"),
            ],
            [
              t("skill.inputs"),
              skill.inputs.length > 0 ? skill.inputs.map((input) => input.name).join(", ") : "—",
            ],
            [
              t("skill.timeout"),
              skill.timeoutSeconds != null ? t("skill.seconds", { n: skill.timeoutSeconds }) : "—",
            ],
            [
              t("col.requiredBy"),
              skill.requiredBy.length > 0
                ? skill.requiredBy.map((trigger, index) => (
                    <span key={trigger.id}>
                      {index > 0 ? ", " : null}
                      <Lk to="/triggers">{trigger.name}</Lk>
                    </span>
                  ))
                : "—",
            ],
          ]}
        />
        <Section count={t("skill.days", { n: stats.days })}>{t("skill.reliability")}</Section>
        <Kv
          items={[
            [t("skill.runs"), stats.runs],
            [
              t("status.succeeded"),
              stats.runs > 0
                ? t("skill.succeededOf", {
                    pct: percent(stats.succeeded / stats.runs),
                    n: stats.succeeded,
                    total: stats.runs,
                  })
                : "—",
            ],
            [
              t("col.medianTime"),
              stats.medianDurationSeconds != null ? durationLong(stats.medianDurationSeconds) : "—",
            ],
            [t("skill.medianFixRounds"), stats.medianHandoffs ?? "—"],
            [
              t("skill.gaveUp"),
              stats.gaveUp > 0
                ? [
                    stats.gaveUp,
                    ...stats.failureErrors.map((error) => error.replace(/^Skill\./, "")),
                  ].join(" · ")
                : "0",
            ],
          ]}
        />
        {skill.openChangeRequest ? (
          <>
            <Section>{t("skill.pendingChange")}</Section>
            <State tone="wait" icon={GitPullRequest}>
              {t("triggers.changeRequestOpen", { n: skill.openChangeRequest.number })}
            </State>
            <Dim className="mb-2 mt-1">
              {skill.openChangeRequest.title}.{" "}
              {skill.openChangeRequest.validation.valid
                ? t("skill.validationPassed")
                : t("skill.validationFailed")}
            </Dim>
            <Button small icon={ExternalLink} href={skill.openChangeRequest.url}>
              {t("common.reviewOnGitHub")}
            </Button>
          </>
        ) : null}
      </>
    )
    body = <Cols main={main} aside={aside} />
  }

  return (
    <Page
      crumbs={[{ label: t("nav.skills"), to: "/skills" }, { label: name }]}
      actions={
        skill?.editUrl ? (
          <Button variant="primary" icon={ExternalLink} kbd="e" href={skill.editUrl}>
            {t("common.editOnGitHub")}
          </Button>
        ) : null
      }
      keys={[
        ...(skill?.editUrl ? [["e", "common.editOnGitHub"] as [string, MessageKey]] : []),
        ["Esc", "keys.back"],
      ]}
    >
      {body}
    </Page>
  )
}
