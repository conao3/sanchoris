import { useNavigate, useParams } from "@tanstack/react-router"
import clsx from "clsx"
import { MessageSquare, Square } from "lucide-react"
import { type FormEvent, useEffect, useRef, useState } from "react"
import { Button } from "../components/Button.tsx"
import { ConfirmDialog } from "../components/ConfirmDialog.tsx"
import { Cols, Page } from "../components/Shell.tsx"
import { RunState, SkillRunState } from "../components/status.tsx"
import {
  ActionError,
  Dim,
  Empty,
  ErrorNote,
  ExternalLk,
  Kv,
  Lk,
  Row,
  Section,
  Skeleton,
  Sub,
  Title,
} from "../components/ui.tsx"
import {
  type LogLineKind,
  RunDocument,
  RunLogDocument,
  type RunLogQuery,
  type RunStatus,
  SendChatMessageDocument,
  StopRunDocument,
  type WorkspaceArchitecture,
} from "../graphql/generated/graphql.ts"
import { type MessageKey, t } from "../i18n/index.ts"
import { clock, dayTime } from "../lib/format.ts"
import { useKeys } from "../lib/keys.ts"
import { useAction } from "../lib/mutation.ts"
import { LIVE_POLL, useData } from "../lib/query.ts"
import { useTenant } from "../lib/tenant.tsx"

const lineClass: Record<LogLineKind, string> = {
  SYSTEM: "",
  INSTRUCTION: "bg-term-instruction-bg text-term-instruction",
  AGENT: "text-white",
  TOOL: "",
  EVENT: "text-term-event",
  USER: "text-term-user",
  ERROR: "text-term-error",
}

const architectureName: Record<WorkspaceArchitecture, string> = {
  ARM64: "arm64",
  X86_64: "x86_64",
}

const linePrefix: Partial<Record<LogLineKind, MessageKey>> = {
  INSTRUCTION: "run.prefixSanchoris",
  AGENT: "run.prefixAgent",
  TOOL: "run.prefixTool",
  USER: "run.prefixYou",
}

const rateLimitText: Record<string, MessageKey> = {
  allowed: "run.rateLimitAllowed",
  allowed_warning: "run.rateLimitWarning",
  rejected: "run.rateLimitRejected",
}

function lineText(line: {
  text: string
  rateLimit: { status: string; resetsAt: string | null } | null
}): string {
  if (!line.rateLimit) {
    return line.text
  }
  const { status, resetsAt } = line.rateLimit
  const key = rateLimitText[status]
  const text = key ? t(key) : t("run.rateLimitOther", { status })
  return resetsAt ? t("run.rateLimitResets", { text, at: dayTime(resetsAt) }) : text
}

type LogLine = NonNullable<
  NonNullable<NonNullable<RunLogQuery["tenant"]>["task"]>["run"]
>["log"][number]

function isLive(status: RunStatus | undefined): boolean {
  return status === "RUNNING" || status === "IDLE" || status === "CREATING"
}

function useRunLog(
  variables: { tenant: string; task: string; number: number },
  live: boolean,
): LogLine[] {
  const [log, setLog] = useState<LogLine[]>([])
  const after = log.at(-1)?.sequence ?? null
  const { data } = useData(
    RunLogDocument,
    { ...variables, after },
    { poll: () => (live ? LIVE_POLL : 0) },
  )
  const fresh = data?.tenant?.task?.run?.log
  const wasLive = useRef(live)

  useEffect(() => {
    if (fresh && fresh.length > 0) {
      setLog((current) => {
        if (after === null) {
          return fresh
        }
        const last = current.at(-1)?.sequence ?? 0
        return [...current, ...fresh.filter((line) => line.sequence > last)]
      })
    }
  }, [fresh, after])

  useEffect(() => {
    if (wasLive.current && !live) {
      setLog([])
    }
    wasLive.current = live
  }, [live])

  return log
}

export function RunPage() {
  const { task, n } = useParams({ from: "/app/tasks/$task/runs/$n" })
  return <RunView key={`${task}/${n}`} taskId={task} n={n} />
}

function RunView({ taskId, n }: { taskId: string; n: string }) {
  const { slug } = useTenant()
  const navigate = useNavigate()
  const [follow, setFollow] = useState(true)
  const variables = { tenant: slug, task: taskId, number: Number(n) }
  const { data, error, refetch } = useData(RunDocument, variables, {
    poll: (result) => (isLive(result.tenant?.task?.run?.status) ? LIVE_POLL : 0),
  })
  const [stopRun, stopState] = useAction(StopRunDocument)
  const [confirmingStop, setConfirmingStop] = useState(false)
  const [send, sendState] = useAction(SendChatMessageDocument)
  const [text, setText] = useState("")
  const input = useRef<HTMLInputElement>(null)
  const logBox = useRef<HTMLDivElement>(null)
  const task = data?.tenant?.task
  const run = task?.run
  const live = isLive(run?.status)
  const log = useRunLog(variables, live)

  useEffect(() => {
    if (follow) {
      logBox.current?.scrollTo({ top: logBox.current.scrollHeight })
    }
  }, [follow, log.length])

  function submit(event?: FormEvent) {
    event?.preventDefault()
    const tenantId = data?.tenant?.id
    if (!task || !tenantId || !text.trim()) {
      return
    }
    void send({ input: { tenantId, taskId: task.id, text: text.trim() } }).then((done) => {
      if (!done) {
        return
      }
      setText("")
      refetch()
    })
  }

  function askStop() {
    if (run && live) {
      setConfirmingStop(true)
    }
  }

  function confirmStop() {
    setConfirmingStop(false)
    if (run && live) {
      void stopRun({ runId: run.id })
    }
  }

  useKeys({
    Escape: () => void navigate({ to: "/tasks/$task", params: { task: taskId } }),
    f: () => setFollow(!follow),
    m: () => input.current?.focus(),
    s: askStop,
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
        <Skeleton rows={8} />
      </div>
    )
  } else if (!task || !run) {
    body = (
      <div className="px-4 py-6 md:px-6">
        <Empty>{t("run.notFound")}</Empty>
      </div>
    )
  } else {
    const agent = t(run.agent === "CODEX" ? "provider.codex" : "provider.claude")
    const main = (
      <div className="flex h-full min-h-[480px] flex-col max-lg:h-[calc(100dvh-5.25rem)]">
        <Title>{t("run.title", { n: run.number })}</Title>
        <Sub>
          <RunState status={run.status} />{" "}
          <span className="ml-2">
            {t("run.sessionLine", {
              session: run.sessionName,
              agent,
              model: run.model,
              at: clock(run.startedAt),
            })}{" "}
            <Lk to="/tasks/$task" params={{ task: task.id }}>
              {task.issue ? `#${task.issue.number} ` : ""}
              {task.title}
            </Lk>
          </span>
        </Sub>
        <ActionError error={stopState.error} className="-mt-3 mb-3.5" />
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-ui bg-term-bg py-3 font-mono text-sm leading-[1.75] text-term-fg">
          <div ref={logBox} className="min-h-0 flex-1 overflow-auto">
            {log.map((line, index) => {
              const [first, ...rest] = lineText(line).split("\n")
              const prefix = linePrefix[line.kind]
              const current = index === log.length - 1
              return (
                <div
                  key={line.sequence}
                  className={clsx(lineClass[line.kind], current && "bg-term-line")}
                >
                  <div className="whitespace-pre-wrap px-4">
                    <span className="text-muted">{clock(line.at)}</span>{" "}
                    {prefix ? `${t(prefix)} ` : ""}
                    {first}
                  </div>
                  {rest.map((continuation, offset) => (
                    <div key={offset} className="whitespace-pre-wrap px-4 pl-[calc(1rem+6ch)]">
                      {continuation}
                    </div>
                  ))}
                </div>
              )
            })}
          </div>
          <form
            onSubmit={submit}
            className="mt-2.5 flex items-center justify-between gap-3 border-t border-term-line px-4 pt-2.5 text-muted"
          >
            <input
              ref={input}
              value={text}
              onChange={(event) => setText(event.target.value)}
              placeholder={t("run.messagePlaceholder")}
              aria-label={t("run.messagePlaceholder")}
              disabled={!live || sendState.loading}
              className="min-w-0 flex-1 bg-transparent text-term-fg outline-none placeholder:text-muted"
            />
            <span>{t("run.enterSend")}</span>
          </form>
        </div>
        <ActionError error={sendState.error} className="mt-2" />
      </div>
    )
    const prs = run.outputs.filter((output) => output.kind === "PULL_REQUEST")
    const aside = (
      <>
        <Section first>{t("run.session")}</Section>
        <Kv
          items={[
            [t("run.session"), <span className="font-mono text-sm">{run.sessionName}</span>],
            [
              t("run.worker"),
              <>
                {agent} · <span className="font-mono text-sm">{run.model}</span>
              </>,
            ],
            [t("run.credential"), <span className="font-mono text-sm">{run.credentialName}</span>],
            [
              t("run.repos"),
              <span className="font-mono text-sm">{run.workspace.repositories.join(", ")}</span>,
            ],
            [t("run.branch"), <span className="font-mono text-sm">{run.workspace.branch}</span>],
            [
              t("run.workspace"),
              run.workspace.capacityProvider === null
                ? t("run.workspaceSize", {
                    platform: run.workspace.platform,
                    architecture: architectureName[run.workspace.architecture],
                    vcpu: run.workspace.vcpu,
                  })
                : t("run.workspaceSizeOn", {
                    platform: run.workspace.platform,
                    capacity: run.workspace.capacityProvider,
                    architecture: architectureName[run.workspace.architecture],
                    vcpu: run.workspace.vcpu,
                  }),
            ],
          ]}
        />
        <Section count={run.skillRuns.length}>{t("run.skillsCalled")}</Section>
        {run.skillRuns.length === 0 ? <Dim>—</Dim> : null}
        {run.skillRuns.map((skillRun) => (
          <Row key={skillRun.id} cols="1fr auto">
            <span>
              <span className="font-mono font-medium">{skillRun.skill.name}</span>{" "}
              <span className="text-muted">{t("run.fixRounds", { n: skillRun.handoffs })}</span>
            </span>
            <SkillRunState status={skillRun.status} />
          </Row>
        ))}
        <Section>{t("run.sessionTotal")}</Section>
        <Kv
          items={[
            [t("col.turns"), <span className="font-mono text-sm">{run.turns}</span>],
            [t("col.cost"), <span className="font-mono text-sm">${run.costUsd.toFixed(2)}</span>],
          ]}
        />
        <Section>{t("run.output")}</Section>
        {prs.length === 0 ? <Dim>—</Dim> : null}
        {prs.map((output) => (
          <div key={output.id}>
            <ExternalLk href={output.url}>
              {output.repository}#{output.number}
            </ExternalLk>
            , {t(`output.${output.state.toLowerCase()}` as MessageKey)}
          </div>
        ))}
      </>
    )
    body = <Cols main={main} aside={aside} />
  }

  const keys: [string, MessageKey][] = [
    ["Esc", "keys.back"],
    ["f", follow ? "run.followOff" : "run.follow"],
    ["m", "run.sendMessage"],
  ]
  if (live) {
    keys.push(["s", "run.stop"])
  }

  return (
    <Page
      crumbs={[
        { label: t("nav.tasks"), to: "/tasks" },
        { label: task?.title ?? "…", to: "/tasks/$task", params: { task: taskId } },
        { label: t("run.title", { n }) },
      ]}
      actions={
        <>
          <Button
            variant="danger"
            icon={Square}
            kbd="s"
            compact
            isDisabled={!live || stopState.loading}
            onPress={askStop}
          >
            {t("run.stop")}
          </Button>
          <Button
            icon={MessageSquare}
            kbd="m"
            compact
            isDisabled={!live}
            onPress={() => input.current?.focus()}
          >
            {t("run.sendMessage")}
          </Button>
        </>
      }
      keys={keys}
    >
      {body}
      <ConfirmDialog
        isOpen={confirmingStop}
        onOpenChange={setConfirmingStop}
        title={t("run.stopTitle")}
        confirmLabel={t("run.stop")}
        confirmIcon={Square}
        onConfirm={confirmStop}
      >
        {t("run.stopBody")}
      </ConfirmDialog>
    </Page>
  )
}
