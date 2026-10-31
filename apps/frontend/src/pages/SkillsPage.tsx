import { useNavigate } from "@tanstack/react-router"
import { FileCode, Plus, ShieldCheck } from "lucide-react"
import { Button } from "../components/Button.tsx"
import { Page, Pg } from "../components/Shell.tsx"
import {
  Dim,
  Empty,
  ErrorNote,
  Lk,
  Row,
  Skeleton,
  State,
  Sub,
  Table,
  Th,
  Title,
} from "../components/ui.tsx"
import { SkillsDocument } from "../graphql/generated/graphql.ts"
import { type MessageKey, t, tx } from "../i18n/index.ts"
import { duration, percent } from "../lib/format.ts"
import { listKeys, useListNav } from "../lib/keys.ts"
import { useData } from "../lib/query.ts"
import { useTenant } from "../lib/tenant.tsx"

export function SkillsPage() {
  const { slug } = useTenant()
  const navigate = useNavigate()
  const { data, error, refetch } = useData(SkillsDocument, { tenant: slug })
  const skills = data?.tenant?.skills ?? []
  const newSkillUrl = data?.tenant?.configs[0]?.newSkillUrl

  function open(index: number) {
    const skill = skills[index]
    if (skill) {
      void navigate({ to: "/skills/$skill", params: { skill: skill.name } })
    }
  }

  const nav = useListNav(
    skills.length,
    open,
    newSkillUrl ? { n: () => void window.open(newSkillUrl, "_blank", "noreferrer") } : {},
  )
  const cols = "170px 110px 1fr 190px 70px 70px 90px"

  return (
    <Page
      crumbs={[{ label: t("nav.skills") }]}
      actions={
        newSkillUrl ? (
          <Button variant="primary" icon={Plus} kbd="n" href={newSkillUrl}>
            {t("skills.add")}
          </Button>
        ) : null
      }
      keys={[
        ...listKeys(skills.length, "keys.open"),
        ...(newSkillUrl ? [["n", "skills.add"] as [string, MessageKey]] : []),
      ]}
    >
      <Pg>
        <Title>{t("nav.skills")}</Title>
        <Sub>{t("skills.intro")}</Sub>
        {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
        {!data && !error ? <Skeleton rows={3} /> : null}
        {data && skills.length === 0 ? <Empty>{t("skills.empty")}</Empty> : null}
        {skills.length > 0 ? (
          <Table>
            <Th cols={cols}>
              <span>{t("col.skill")}</span>
              <span>{t("col.kind")}</span>
              <span>{t("col.whenCalled")}</span>
              <span>{t("col.requiredBy")}</span>
              <span className="text-right">{t("col.runs30d")}</span>
              <span className="text-right">{t("col.success")}</span>
              <span className="text-right">{t("col.medianTime")}</span>
            </Th>
            {skills.map((skill, index) => {
              const { stats } = skill
              return (
                <Row
                  key={skill.id}
                  cols={cols}
                  tall
                  selected={nav.selected === index}
                  rowRef={nav.rowRef(index)}
                  onClick={() => open(index)}
                >
                  <Lk
                    to="/skills/$skill"
                    params={{ skill: skill.name }}
                    className="font-mono font-medium"
                  >
                    {skill.name}
                  </Lk>
                  {skill.kind === "RELIABLE" ? (
                    <State tone="ok" icon={ShieldCheck}>
                      {t("skills.reliable")}
                    </State>
                  ) : (
                    <State tone="wait" icon={FileCode}>
                      {t("skills.plain")}
                    </State>
                  )}
                  <span className="text-muted">{skill.description}</span>
                  {skill.requiredBy.length > 0 ? (
                    <span className="font-mono text-sm">
                      {skill.requiredBy.map((trigger) => trigger.name).join(", ")}
                    </span>
                  ) : (
                    <span className="text-muted">—</span>
                  )}
                  <span className="text-right font-mono text-sm">
                    {stats.runs > 0 ? stats.runs : "—"}
                  </span>
                  <span className="text-right font-mono text-sm">
                    {stats.runs > 0 ? percent(stats.succeeded / stats.runs) : "—"}
                  </span>
                  <span className="text-right font-mono text-sm">
                    {stats.medianDurationSeconds != null
                      ? duration(stats.medianDurationSeconds)
                      : "—"}
                  </span>
                </Row>
              )
            })}
          </Table>
        ) : null}
        <Dim className="mt-3.5">
          {tx("skills.footer", {
            skillMd: <span className="font-mono text-sm">SKILL.md</span>,
            skillYaml: <span className="font-mono text-sm">skill.yaml</span>,
          })}
        </Dim>
      </Pg>
    </Page>
  )
}
