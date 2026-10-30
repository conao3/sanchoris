import { useNavigate, useParams } from "@tanstack/react-router"
import clsx from "clsx"
import type { LucideIcon } from "lucide-react"
import {
  Bot,
  Check,
  ChevronRight,
  Circle,
  CircleCheck,
  CircleDashed,
  CircleX,
  Play,
  Puzzle,
  Square,
  X,
  Zap,
} from "lucide-react"
import { type ReactNode, useState } from "react"
import { Button } from "../components/Button.tsx"
import { ConfirmDialog } from "../components/ConfirmDialog.tsx"
import { Cols, Page } from "../components/Shell.tsx"
import { SkillRunState, StateEntryState, TaskState } from "../components/status.tsx"
import {
  ActionError,
  Dim,
  Empty,
  ErrorNote,
  ExternalLk,
  Icon,
  Id,
  Kv,
  Lk,
  Meter,
  Row,
  Section,
  Skeleton,
  State,
  Sub,
  Table,
  Th,
  Title,
  type Tone,
} from "../components/ui.tsx"
import {
  ResolveApprovalDocument,
  ResumeTaskDocument,
  type RunStatus,
  type StateType,
  StopTaskDocument,
  type TaskQuery,
  TaskDocument,
} from "../graphql/generated/graphql.ts"
import { type MessageKey, t } from "../i18n/index.ts"
import { between, clock, duration, usd, usdWhole } from "../lib/format.ts"
import { useKeys } from "../lib/keys.ts"
import { useAction } from "../lib/mutation.ts"
import { LIVE_POLL, useData } from "../lib/query.ts"
import { isActive, needsYouText, skillProgress } from "../lib/taskText.ts"
import { useTenant } from "../lib/tenant.tsx"

type Task = NonNullable<NonNullable<TaskQuery["tenant"]>["task"]>

const stateTypeLabels: Record<StateType, MessageKey> = {
  TASK: "stateType.action",
  PASS: "stateType.pass",
  CHOICE: "stateType.choice",
  WAIT: "stateType.wait",
  SUCCEED: "stateType.succeed",
  FAIL: "stateType.fail",
}

const runStatusLabels: Record<RunStatus, MessageKey> = {
  CREATING: "status.starting",
  RUNNING: "status.running",
  IDLE: "status.idle",
  COMPLETED: "status.completed",
  FAILED: "status.failed",
}

function Step({
  kind,
  icon,
  tone,
  stateIcon,
  children,
  current,
  todo,
}: {
  kind: string
  icon: LucideIcon
  tone: Tone
  stateIcon: LucideIcon
  children: ReactNode
  current?: boolean
  todo?: boolean
}) {
  return (
    <div
      className={clsx(
        "min-w-0 flex-1 rounded-ui border px-[11px] py-2",
        current ? "border-ink shadow-[0_0_0_1px_var(--color-ink)]" : "border-line2",
        todo ? "border-dashed bg-sub" : "bg-bg",
      )}
    >
      <div className="flex items-center gap-1.5 font-mono text-xs font-medium text-muted">
        <Icon icon={icon} className="size-[13px]" />
        {kind}
      </div>
      <State tone={tone} icon={stateIcon} className="mt-[3px]" wrap>
        {children}
      </State>
    </div>
  )
}

function Arrow() {
  return (
    <span className="flex items-center justify-center text-faint max-md:h-5 md:w-[26px]">
      <Icon icon={ChevronRight} className="size-3.5 max-md:rotate-90" />
    </span>
  )
}

function StepStrip({ task }: { task: Task }) {
  const required = task.requiredSkills[0]
  const skillRun = required?.latestRun
    ? task.skillRuns.find((run) => run.id === required.latestRun?.id)
    : undefined
  const done = task.status === "SUCCEEDED"
  const agentWorking = task.status === "RUNNING" && !skillRun
  return (
    <div className="mb-1 mt-0.5 flex items-stretch max-md:flex-col">
      <Step
        kind={t("task.stepTrigger", { name: task.trigger.name })}
        icon={Zap}
        tone="ok"
        stateIcon={CircleCheck}
      >
        {task.startedAt ? t("task.startedAt", { at: clock(task.startedAt) }) : t("task.notStarted")}
      </Step>
      <Arrow />
      <Step
        kind={t("task.stepAgent", { session: task.session.name })}
        icon={Bot}
        tone={agentWorking ? "run" : task.startedAt ? "ok" : "wait"}
        stateIcon={agentWorking ? CircleDashed : task.startedAt ? CircleCheck : Circle}
        current={agentWorking}
        todo={!task.startedAt}
      >
        {!task.startedAt
          ? t("task.notStarted")
          : skillRun
            ? t("task.worked", { from: clock(task.startedAt), to: clock(skillRun.startedAt) })
            : t("task.workingSince", { from: clock(task.startedAt) })}
      </Step>
      {required ? (
        <>
          <Arrow />
          <Step
            kind={t("task.stepSkill", { name: required.skill.name })}
            icon={Puzzle}
            tone={
              required.state === "FAILED"
                ? "err"
                : required.state === "SUCCEEDED"
                  ? "ok"
                  : required.state === "RUNNING"
                    ? "run"
                    : "wait"
            }
            stateIcon={
              required.state === "FAILED"
                ? CircleX
                : required.state === "SUCCEEDED"
                  ? CircleCheck
                  : required.state === "RUNNING"
                    ? CircleDashed
                    : Circle
            }
            current={!!skillRun && !done}
            todo={!skillRun}
          >
            {required.state === "FAILED"
              ? t("task.skillFailed", { error: skillRun?.error ?? "" })
              : required.state === "SUCCEEDED"
                ? t("status.succeeded")
                : required.state === "RUNNING"
                  ? t("task.atState", { state: skillRun?.currentState ?? "" })
                  : t("task.notRun")}
          </Step>
        </>
      ) : null}
      <Arrow />
      <Step
        kind={t("task.stepTask")}
        icon={CircleCheck}
        tone={done ? "ok" : "wait"}
        stateIcon={done ? CircleCheck : Circle}
        todo={!done}
        current={done}
      >
        {done ? t("status.completed") : t("task.notCompleted")}
      </Step>
    </div>
  )
}

export function TaskPage() {
  const { task: taskId } = useParams({ from: "/app/tasks/$task" })
  const { slug } = useTenant()
  const navigate = useNavigate()
  const { data, error, refetch } = useData(
    TaskDocument,
    { tenant: slug, id: taskId },
    {
      poll: (result) =>
        result.tenant?.task && isActive(result.tenant.task.status) ? LIVE_POLL : 0,
    },
  )
  const [message, setMessage] = useState("")
  const [resume, resumeState] = useAction(ResumeTaskDocument)
  const [stop, stopState] = useAction(StopTaskDocument)
  const [confirmingStop, setConfirmingStop] = useState(false)
  const [resolveApproval, approvalState] = useAction(ResolveApprovalDocument)
  const task = data?.tenant?.task
  const needsYou = task?.status === "NEEDS_YOU"
  const approval = task?.approval
  const resumable = needsYou && !approval
  const active = task ? isActive(task.status) : false

  function doResolve(approved: boolean) {
    if (task && approval) {
      void resolveApproval({ taskId: task.id, approved })
    }
  }

  function doResume() {
    if (task && resumable && message.trim()) {
      void resume({ input: { taskId: task.id, message: message.trim() } }).then((done) => {
        if (!done) {
          return
        }
        setMessage("")
        refetch()
      })
    }
  }

  function doStop() {
    if (task && active) {
      setConfirmingStop(true)
    }
  }

  function confirmStop() {
    setConfirmingStop(false)
    if (task && active) {
      void stop({ taskId: task.id })
    }
  }

  useKeys({
    "mod+Enter": doResume,
    s: doStop,
    o: () => {
      const run = task?.runs[task.runs.length - 1]
      if (task && run) {
        void navigate({
          to: "/tasks/$task/runs/$n",
          params: { task: task.id, n: String(run.number) },
        })
      }
    },
    Escape: () => void navigate({ to: "/tasks" }),
  })

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
  } else if (!task) {
    body = (
      <div className="px-4 py-6 md:px-6">
        <Empty>{t("task.notFound")}</Empty>
      </div>
    )
  } else {
    const latestSkillRun = task.skillRuns[task.skillRuns.length - 1]
    const runCols = "24px 150px 96px 72px 56px 72px 1fr"
    const stepCols = "28px 110px 64px 100px 56px 60px 1fr"
    const pr = task.outputs.filter((output) => output.kind === "PULL_REQUEST")
    const relations: [ReactNode, ReactNode][] = [
      [t("col.trigger"), <Lk to="/triggers">{task.trigger.name}</Lk>],
    ]
    if (task.triggerEvent) {
      relations.push([
        t("task.startedBy"),
        task.issue && task.triggerEvent.senderLogin
          ? t("task.issueLabeledBy", {
              login: task.triggerEvent.senderLogin,
              at: clock(task.triggerEvent.receivedAt),
            })
          : t("task.messageAt", { at: clock(task.triggerEvent.receivedAt) }),
      ])
    }
    if (task.requester) {
      relations.push([
        t("task.requestedBy"),
        task.requester.kind === "AGENT" && task.requester.task ? (
          <Lk to="/tasks/$task" params={{ task: task.requester.task.id }}>
            {t("task.agentOf", { title: task.requester.task.title })}
          </Lk>
        ) : task.requester.viaChat ? (
          t("task.inChat", { name: task.requester.member?.user.name ?? "" })
        ) : (
          (task.requester.member?.user.name ?? "")
        ),
      ])
    }
    if (task.parent) {
      relations.push([
        t("task.parent"),
        <Lk to="/tasks/$task" params={{ task: task.parent.id }}>
          {task.parent.issue ? `#${task.parent.issue.number}` : task.parent.title}
        </Lk>,
      ])
    }
    if (task.children.length > 0) {
      relations.push([
        t("task.children"),
        task.children.map((child, index) => (
          <span key={child.id}>
            {index > 0 ? ", " : null}
            <Lk to="/tasks/$task" params={{ task: child.id }}>
              {child.issue ? `#${child.issue.number}` : child.title}
            </Lk>
          </span>
        )),
      ])
    }
    if (task.blockedBy.length > 0) {
      relations.push([
        t("task.blockedByLabel"),
        task.blockedBy.map((blocker) => `#${blocker.issue?.number ?? ""}`).join(", "),
      ])
    }
    relations.push(
      [
        t("col.session"),
        <>
          <span className="font-mono text-sm">{task.session.name}</span> ·{" "}
          {t(task.session.agent === "CODEX" ? "provider.codex" : "provider.claude")}
        </>,
      ],
      [t("task.nudges"), t("task.nudgesOf", { n: task.nudges, total: task.maxNudges })],
      [
        t("task.version"),
        <span className="font-mono text-sm">{t("task.pinned", { rev: task.configRevision })}</span>,
      ],
    )

    const main = (
      <>
        <Title>{task.title}</Title>
        <Sub>
          <TaskState status={task.status} waitReason={task.waitReason} />{" "}
          {task.issue ? (
            <span className="ml-2">
              <Id>
                {task.issue.repository}#{task.issue.number}
              </Id>
            </span>
          ) : null}{" "}
          ·{" "}
          {needsYou
            ? needsYouText(task)
            : `${skillProgress(task)} · ${t("task.spent", { cost: usd(task.costUsd) })}`}
        </Sub>
        <ActionError error={stopState.error} className="-mt-3 mb-3.5" />
        <Section count={t("task.fromTrigger", { name: task.trigger.name })} first>
          {task.requiredSkills.length > 0 ? t("task.requiredSkill") : t("task.progress")}
        </Section>
        <StepStrip task={task} />

        {needsYou && approval ? (
          <>
            <Section>{t("status.needsYou")}</Section>
            <div className="font-medium">{approval.message}</div>
            <Dim className="mt-1">{t("task.approvalNote")}</Dim>
            <ActionError error={approvalState.error} className="mt-2" />
            <div className="mt-2.5 flex gap-2">
              <Button
                variant="primary"
                icon={Check}
                onPress={() => doResolve(true)}
                isDisabled={approvalState.loading}
              >
                {t("task.approve")}
              </Button>
              <Button icon={X} onPress={() => doResolve(false)} isDisabled={approvalState.loading}>
                {t("task.reject")}
              </Button>
              <Button
                variant="danger"
                icon={Square}
                kbd="s"
                onPress={doStop}
                isDisabled={stopState.loading}
              >
                {t("task.stop")}
              </Button>
            </div>
          </>
        ) : null}

        {resumable ? (
          <>
            <Section>{t("status.needsYou")}</Section>
            <Dim>
              {task.cause ?? task.error} {t("task.messageGoes")}
            </Dim>
            <label htmlFor="resume-message" className="mb-1 mt-3 block text-xs text-muted">
              {t("task.messageToAgent")}
            </label>
            <textarea
              id="resume-message"
              value={message}
              onChange={(event) => setMessage(event.target.value)}
              placeholder={t("task.messagePlaceholder")}
              className="block h-16 w-full resize-y rounded-ui border border-line2 bg-bg px-2.5 py-[7px] outline-none focus:border-ink"
            />
            <ActionError error={resumeState.error} className="mt-2" />
            <div className="mt-2.5 flex gap-2">
              <Button
                variant="primary"
                icon={Play}
                kbd="⌘↵"
                onPress={doResume}
                isDisabled={!message.trim() || resumeState.loading}
              >
                {t("task.resume")}
              </Button>
              <Button
                variant="danger"
                icon={Square}
                kbd="s"
                onPress={doStop}
                isDisabled={stopState.loading}
              >
                {t("task.stop")}
              </Button>
            </div>
          </>
        ) : null}

        <Section count={task.skillRuns.length}>{t("task.skillRuns")}</Section>
        {task.skillRuns.length === 0 ? <Dim>{t("task.noSkillRuns")}</Dim> : null}
        {task.skillRuns.length > 0 ? (
          <Table>
            <Th cols={runCols}>
              <span>#</span>
              <span>{t("col.skill")}</span>
              <span>{t("col.status")}</span>
              <span className="text-right">{t("col.fixRounds")}</span>
              <span>{t("col.started")}</span>
              <span className="text-right">{t("col.duration")}</span>
              <span>{t("col.error")}</span>
            </Th>
            {task.skillRuns.map((run) => (
              <Row key={run.id} cols={runCols} selected={run.id === latestSkillRun?.id}>
                <span className="text-right font-mono text-sm">{run.number}</span>
                <span className="font-mono font-medium">{run.skill.name}</span>
                <SkillRunState status={run.status} />
                <span className="text-right font-mono text-sm">{run.handoffs}</span>
                <span className="font-mono text-sm">{clock(run.startedAt)}</span>
                <span className="text-right font-mono text-sm">
                  {run.endedAt ? duration(between(run.startedAt, run.endedAt)) : "—"}
                </span>
                <span className="text-muted">
                  {run.error ? [run.error, run.cause].filter(Boolean).join(" · ") : "—"}
                </span>
              </Row>
            ))}
          </Table>
        ) : null}

        {latestSkillRun && latestSkillRun.stateEntries.length > 0 ? (
          <>
            <Section
              count={t("task.lastOf", {
                n: latestSkillRun.stateEntries.length,
                total: latestSkillRun.stateEntryCount,
              })}
            >
              {t("task.stepsOfRun", { n: latestSkillRun.number })}
            </Section>
            <Table>
              <Th cols={stepCols}>
                <span>#</span>
                <span>{t("col.state")}</span>
                <span>{t("col.type")}</span>
                <span>{t("col.status")}</span>
                <span>{t("col.entered")}</span>
                <span className="text-right">{t("col.attempts")}</span>
                <span>{t("col.result")}</span>
              </Th>
              {latestSkillRun.stateEntries.map((entry) => (
                <Row key={entry.id} cols={stepCols}>
                  <span className="text-right font-mono text-sm">{entry.sequence}</span>
                  <span className="font-medium">{entry.stateName}</span>
                  <span>{t(stateTypeLabels[entry.stateType])}</span>
                  <StateEntryState status={entry.status} />
                  <span className="font-mono text-sm">{clock(entry.enteredAt)}</span>
                  <span className="text-right font-mono text-sm">{entry.attempts}</span>
                  <span className="text-muted wrap-anywhere">{entry.result ?? "—"}</span>
                </Row>
              ))}
            </Table>
          </>
        ) : null}
      </>
    )

    const aside = (
      <>
        <Section first>{t("task.relations")}</Section>
        <Kv items={relations} />
        <Section count={task.runs.length}>{t("task.runs")}</Section>
        {task.runs.map((run) => (
          <Row key={run.id} cols="44px 1fr">
            <Lk
              to="/tasks/$task/runs/$n"
              params={{ task: task.id, n: String(run.number) }}
              className="font-mono text-sm"
            >
              {t("run.title", { n: run.number })}
            </Lk>
            <span>
              <span className="font-mono text-sm">{run.sessionName}</span> ·{" "}
              {t(run.agent === "CODEX" ? "provider.codex" : "provider.claude")} ·{" "}
              {t(runStatusLabels[run.status]).toLowerCase()}
            </span>
          </Row>
        ))}
        <Section>{t("task.outputs")}</Section>
        {pr.length === 0 ? <Dim>—</Dim> : null}
        {pr.length > 0 ? (
          <Kv
            items={pr.map((output) => [
              t("task.pr"),
              <>
                <ExternalLk href={output.url}>
                  {output.repository}#{output.number}
                </ExternalLk>
                , {t(`output.${output.state.toLowerCase()}` as MessageKey)}
              </>,
            ])}
          />
        ) : null}
        <Section>{t("task.budget")}</Section>
        <div className="mt-2.5">
          <Meter
            value={Math.max(task.costUsd / task.limits.costUsd, task.turns / task.limits.turns)}
          />
        </div>
        <Dim className="mt-1.5">
          {t("task.budgetOf", {
            cost: usd(task.costUsd),
            limit: usdWhole(task.limits.costUsd),
            turns: task.turns,
            maxTurns: task.limits.turns,
          })}
        </Dim>
      </>
    )
    body = <Cols main={main} aside={aside} />
  }

  const keys: [string, MessageKey][] = []
  if (resumable) {
    keys.push(["⌘↵", "task.resume"])
  }
  if (active) {
    keys.push(["s", "task.stop"])
  }
  if (task && task.runs.length > 0) {
    keys.push(["o", "keys.openRun"])
  }
  keys.push(["Esc", "keys.back"])

  return (
    <Page
      crumbs={[{ label: t("nav.tasks"), to: "/tasks" }, { label: task?.title ?? "…" }]}
      actions={
        task ? (
          <>
            {active ? (
              <Button
                variant="danger"
                icon={Square}
                kbd="s"
                compact
                onPress={doStop}
                isDisabled={stopState.loading}
              >
                {t("task.stop")}
              </Button>
            ) : null}
            {resumable ? (
              <Button
                variant="primary"
                icon={Play}
                kbd="⌘↵"
                compact
                onPress={doResume}
                isDisabled={!message.trim()}
              >
                {t("task.resume")}
              </Button>
            ) : null}
          </>
        ) : null
      }
      keys={keys}
    >
      {body}
      <ConfirmDialog
        isOpen={confirmingStop}
        onOpenChange={setConfirmingStop}
        title={t("task.stopTitle")}
        confirmLabel={t("task.stop")}
        confirmIcon={Square}
        onConfirm={confirmStop}
      >
        {t("task.stopBody")}
      </ConfirmDialog>
    </Page>
  )
}
