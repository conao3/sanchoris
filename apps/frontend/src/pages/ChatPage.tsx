import { Link, useNavigate, useParams } from "@tanstack/react-router"
import { CircleDot, Plus, Square } from "lucide-react"
import { type KeyboardEvent, useEffect, useRef, useState } from "react"
import { Button } from "../components/Button.tsx"
import { ChangedFiles } from "../components/chat/ChangedFiles.tsx"
import { ApprovalComposer } from "../components/chat/ApprovalComposer.tsx"
import { DEFAULT_MODES, ModeControls, type Modes } from "../components/chat/ModeControls.tsx"
import { ProposedPlanCard, TasksBanner } from "../components/chat/Plan.tsx"
import { QuestionComposer } from "../components/chat/QuestionComposer.tsx"
import { QueuedBanner } from "../components/chat/QueuedBanner.tsx"
import { AgentMessage, MemberMessage, Working } from "../components/chat/Timeline.tsx"
import { Reason } from "../components/Reason.tsx"
import { Page } from "../components/Shell.tsx"
import { TaskState } from "../components/status.tsx"
import {
  ActionError,
  Chip,
  Dim,
  ErrorNote,
  Icon,
  Id,
  Kbd,
  Lk,
  Row,
  Section,
  Skeleton,
  Sub,
  Table,
  Th,
  Title,
} from "../components/ui.tsx"
import {
  type ChatDelivery,
  type ChatQuery,
  RemoveQueuedChatMessageDocument,
  SteerChatMessageDocument,
  AnswerChatApprovalDocument,
  AnswerChatQuestionDocument,
  ChatDocument,
  InterruptChatDocument,
  SendChatMessageDocument,
} from "../graphql/generated/graphql.ts"
import { t, tx } from "../i18n/index.ts"
import { useKeys } from "../lib/keys.ts"
import { useAction } from "../lib/mutation.ts"
import { IDLE_POLL, LIVE_POLL, useData } from "../lib/query.ts"
import { isActive, skillStep } from "../lib/taskText.ts"
import { useTenant } from "../lib/tenant.tsx"

type Tenant = NonNullable<ChatQuery["tenant"]>
type ChatSession = Tenant["sessions"][number]
type ChatConversation = NonNullable<Tenant["conversation"]>

function shortRepo(repository: string): string {
  return repository.split("/").pop() ?? repository
}

function sessionDetail(session: ChatSession): string {
  return t("chat.sessionDetail", {
    agent: t(session.agent === "CODEX" ? "provider.codex" : "provider.claude"),
    repos: session.repositories.map(shortRepo).join(", "),
    github: session.githubRepositories.join(", "),
  })
}

function sessionSkills(session: ChatSession): string {
  return session.callableSkills.length > 0
    ? t("chat.sessionSkills", {
        skills: session.callableSkills.map((skill) => skill.name).join(", "),
      })
    : t("chat.noSkills")
}

function Composer({
  reason,
  modes,
  onModesChange,
  onSend,
  onStop,
}: {
  reason?: string
  modes: Modes
  onModesChange: (modes: Modes) => void
  onSend: (text: string, delivery: ChatDelivery) => Promise<boolean>
  onStop?: () => void
}) {
  const [text, setText] = useState("")

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault()
      if (text.trim() && !reason) {
        void onSend(text.trim(), event.metaKey || event.ctrlKey ? "STEER" : "QUEUE").then(
          (sent) => {
            if (sent) {
              setText("")
            }
          },
        )
      }
    }
  }

  return (
    <div className="mt-3.5 rounded-ui border border-line2">
      <div className="flex px-2.5 pt-2">
        <Reason reason={reason} className="flex min-w-0 flex-1">
          <textarea
            rows={1}
            value={text}
            disabled={!!reason}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={onKeyDown}
            placeholder={t("chat.placeholder")}
            aria-label={t("chat.placeholder")}
            className="min-h-5 flex-1 resize-none bg-transparent outline-none placeholder:text-faint"
          />
        </Reason>
      </div>
      <div className="flex items-center justify-between gap-3 px-2 pt-1 pb-1.5">
        {reason ? <span /> : <ModeControls modes={modes} onChange={onModesChange} />}
        {onStop ? (
          <span className="flex items-center gap-2">
            <Kbd>{t("chat.enterQueue")}</Kbd>
            <Button small icon={Square} onPress={onStop}>
              {t("chat.stop")}
            </Button>
          </span>
        ) : reason ? null : (
          <Kbd>{t("chat.enterSend")}</Kbd>
        )}
      </div>
    </div>
  )
}

function Thread({ conversation, tenant }: { conversation: ChatConversation; tenant: Tenant }) {
  const [send, { error }] = useAction(SendChatMessageDocument, { refetchQueries: [ChatDocument] })
  const [answer, { error: answerError }] = useAction(AnswerChatQuestionDocument, {
    refetchQueries: [ChatDocument],
  })
  const [interrupt, { error: interruptError }] = useAction(InterruptChatDocument, {
    refetchQueries: [ChatDocument],
  })
  const [decide, { error: decideError }] = useAction(AnswerChatApprovalDocument, {
    refetchQueries: [ChatDocument],
  })
  const [steer, { error: steerError }] = useAction(SteerChatMessageDocument, {
    refetchQueries: [ChatDocument],
  })
  const [unqueue, { error: unqueueError }] = useAction(RemoveQueuedChatMessageDocument, {
    refetchQueries: [ChatDocument],
  })
  const queued = conversation.messages.filter((message) => message.delivery === "QUEUED")
  const timeline = conversation.messages.filter((message) => message.delivery !== "QUEUED")
  const [modes, setModes] = useState<Modes>({
    interactionMode: conversation.interactionMode,
    runtimeMode: conversation.runtimeMode,
  })
  const working = conversation.working
  const end = useRef<HTMLDivElement>(null)
  const follow = `${conversation.id}:${conversation.messages.length}:${working?.work.length ?? -1}:${working?.question?.id ?? ""}:${working?.approval?.id ?? ""}`
  useEffect(() => {
    end.current?.scrollIntoView({ block: "nearest" })
  }, [follow])
  const task = conversation.latestTask
  const run = task?.runs[task.runs.length - 1]
  const repos = conversation.session?.repositories.join(", ") ?? ""
  const starter = conversation.createdBy?.user.name
  const finished = task ? !isActive(task.status) : false

  function sendText(text: string, using: Modes, delivery: ChatDelivery) {
    return send({
      input: { tenantId: tenant.id, conversationId: conversation.id, text, delivery, ...using },
    }).then(Boolean)
  }

  function implement(markdown: string) {
    const using: Modes = { ...modes, interactionMode: "DEFAULT" }
    setModes(using)
    void sendText(`PLEASE IMPLEMENT THIS PLAN:\n${markdown}`, using, "QUEUE")
  }
  return (
    <>
      <Title>{conversation.title}</Title>
      <Sub>
        {task ? <TaskState status={task.status} waitReason={task.waitReason} /> : null}{" "}
        <span className="ml-2">
          {tx("chat.threadLine", {
            session: <span className="font-mono text-sm">{conversation.sessionName}</span>,
            repos: <span className="font-mono text-sm">{repos}</span>,
            task: task ? (
              <Lk to="/tasks/$task" params={{ task: task.id }}>
                {t("chat.taskN", { n: task.conversationSequence ?? 1 })}
              </Lk>
            ) : (
              "—"
            ),
            run: run ? t("run.title", { n: run.number }) : "—",
          })}
          {starter
            ? ` · ${t(conversation.kind === "SLACK" ? "chat.startedBySlack" : "chat.startedByWeb", { name: starter })}`
            : null}
        </span>
      </Sub>
      <div className="mt-4 space-y-5">
        {timeline.map((message, position) => {
          if (message.authorKind !== "AGENT") {
            return (
              <MemberMessage
                key={message.id}
                name={message.member?.user.name ?? ""}
                text={message.text}
                at={message.at}
                steered={message.delivery === "STEERED"}
              />
            )
          }
          const asked = timeline
            .slice(0, position)
            .findLast((earlier) => earlier.authorKind !== "AGENT")
          return (
            <AgentMessage
              key={message.id}
              text={message.text}
              at={message.at}
              work={message.work}
              startedAt={asked?.at}
              outcome={message.outcome}
            >
              {message.issues.map((issue) => {
                const blockers = issue.task?.blockedBy ?? []
                return (
                  <div
                    key={`${issue.repository}#${issue.number}`}
                    className="mt-1.5 grid grid-cols-[18px_auto_1fr_auto] items-center gap-2 border-t border-line py-1.5"
                  >
                    <Icon icon={CircleDot} />
                    <span>
                      <Chip>{shortRepo(issue.repository)}</Chip>
                    </span>
                    {issue.task ? (
                      <Link
                        to="/tasks/$task"
                        params={{ task: issue.task.id }}
                        className="font-medium"
                      >
                        {issue.title}
                      </Link>
                    ) : (
                      <span className="font-medium">{issue.title}</span>
                    )}
                    <Id>
                      #{issue.number}
                      {blockers.length > 0
                        ? ` · ${t("tasks.blockedBy", { ids: blockers.map((blocker) => `#${blocker.issue?.number ?? ""}`).join(", ") })}`
                        : null}
                    </Id>
                  </div>
                )
              })}
              {message.proposedPlan ? (
                <ProposedPlanCard
                  markdown={message.proposedPlan}
                  onImplement={
                    position === timeline.length - 1 && conversation.kind === "WEB"
                      ? () => implement(message.proposedPlan ?? "")
                      : undefined
                  }
                  implementReason={working ? t("reason.agentWorking") : undefined}
                />
              ) : null}
              {message.diff ? <ChangedFiles diff={message.diff} /> : null}
            </AgentMessage>
          )
        })}
        {working ? (
          <Working
            startedAt={working.startedAt}
            work={working.work}
            waiting={working.approval ? "approval" : working.question ? "question" : undefined}
          />
        ) : null}
      </div>
      {finished && task && conversation.kind === "WEB" ? (
        <Dim className="mt-2.5">
          {t("chat.continueHint", { n: (task.conversationSequence ?? 1) + 1 })}
        </Dim>
      ) : null}
      {queued.length > 0 ? (
        <QueuedBanner
          messages={queued}
          working={!!working}
          onSteer={(messageId) =>
            void steer({ tenantId: tenant.id, conversationId: conversation.id, messageId })
          }
          onRemove={(messageId) =>
            void unqueue({ tenantId: tenant.id, conversationId: conversation.id, messageId })
          }
        />
      ) : null}
      {working?.plan && !working.question && !working.approval ? (
        <TasksBanner plan={working.plan} />
      ) : null}
      {working?.approval ? (
        <ApprovalComposer
          key={working.approval.id}
          approval={working.approval}
          onDecide={(decision) =>
            decide({
              input: {
                tenantId: tenant.id,
                conversationId: conversation.id,
                approvalId: working.approval?.id ?? "",
                decision,
              },
            }).then(Boolean)
          }
        />
      ) : working?.question ? (
        <QuestionComposer
          key={working.question.id}
          question={working.question}
          onStop={() => void interrupt({ tenantId: tenant.id, conversationId: conversation.id })}
          onSubmit={(answers) =>
            answer({
              input: {
                tenantId: tenant.id,
                conversationId: conversation.id,
                questionId: working.question?.id ?? "",
                answers,
              },
            }).then(Boolean)
          }
        />
      ) : (
        <Composer
          reason={conversation.kind === "SLACK" ? t("reason.slackThread") : undefined}
          modes={modes}
          onModesChange={setModes}
          onStop={
            working && run
              ? () => void interrupt({ tenantId: tenant.id, conversationId: conversation.id })
              : undefined
          }
          onSend={(text, delivery) => sendText(text, modes, delivery)}
        />
      )}
      <ActionError
        error={error ?? answerError ?? interruptError ?? decideError ?? steerError ?? unqueueError}
        className="mt-2"
      />
      <div ref={end} />
    </>
  )
}

function SessionPicker({
  sessions,
  selected,
  onSelect,
}: {
  sessions: ChatSession[]
  selected?: ChatSession
  onSelect: (id: string) => void
}) {
  return (
    <Table className="mt-3">
      <Th cols="160px 1fr">
        <span>{t("chat.session")}</span>
        <span>{t("chat.sessionWhat")}</span>
      </Th>
      {sessions.map((session) => (
        <Row
          key={session.id}
          cols="160px 1fr"
          tall
          selected={session.id === selected?.id}
          onClick={session.id === selected?.id ? undefined : () => onSelect(session.id)}
        >
          <span className="font-mono text-sm">{session.name}</span>
          <div>
            <div>{sessionDetail(session)}</div>
            <Dim>{sessionSkills(session)}</Dim>
          </div>
        </Row>
      ))}
    </Table>
  )
}

function NewChat({
  tenant,
  session,
  onSelect,
}: {
  tenant: Tenant
  session?: ChatSession
  onSelect: (id: string) => void
}) {
  const navigate = useNavigate()
  const [send, { error }] = useAction(SendChatMessageDocument, { refetchQueries: [ChatDocument] })
  const [modes, setModes] = useState<Modes>(DEFAULT_MODES)
  const hasConfig = tenant.configs.some((config) => config.status === "ACTIVE")
  return (
    <>
      <Title>{t("chat.newChat")}</Title>
      <Sub>
        {!hasConfig ? (
          <>
            {t("chat.noConfig")} <Lk to="/settings/config">{t("settings.config")}</Lk>
          </>
        ) : tenant.sessions.length === 0 ? (
          <>
            {t("chat.noSessions")} <Lk to="/settings/config">{t("settings.config")}</Lk>
          </>
        ) : session ? (
          <>
            {t("chat.sessionIs", { session: session.name })} · {sessionDetail(session)}
          </>
        ) : (
          t("chat.pickSession")
        )}
      </Sub>
      {tenant.sessions.length > 1 ? (
        <SessionPicker sessions={tenant.sessions} selected={session} onSelect={onSelect} />
      ) : null}
      {session ? <Dim className="mt-2.5">{sessionSkills(session)}</Dim> : null}
      <Composer
        reason={
          !hasConfig || tenant.sessions.length === 0
            ? t("reason.noSession")
            : session
              ? undefined
              : t("reason.pickSession")
        }
        modes={modes}
        onModesChange={setModes}
        onSend={(text) =>
          send({
            input: {
              tenantId: tenant.id,
              sessionName: session?.name,
              configId: session?.config.id,
              text,
              ...modes,
              delivery: "QUEUE",
            },
          }).then((result) => {
            const conversation = result?.sendChatMessage.conversation
            if (conversation) {
              void navigate({
                to: "/chat/$conversation",
                params: { conversation: conversation.id },
              })
            }
            return !!result
          })
        }
      />
      <ActionError error={error} className="mt-2" />
    </>
  )
}

function FiledTasks({ conversation }: { conversation: ChatConversation }) {
  const filedTasks = conversation.filedTasks
  return (
    <>
      <Section count={filedTasks.length} first>
        {t("chat.issuesFiled")}
      </Section>
      <Dim className="mb-1.5">{t("chat.issuesFiledDetail")}</Dim>
      {filedTasks.length === 0 ? <Dim>{t("chat.noIssues")}</Dim> : null}
      {filedTasks.length > 0 ? (
        <Table>
          <Th cols="96px 1fr">
            <span>{t("col.state")}</span>
            <span>{t("col.task")}</span>
          </Th>
          {filedTasks.map((filed) => {
            const run = filed.requiredSkills[0]?.latestRun
            const details = [
              filed.parent?.issue
                ? t("triggers.childOf", { id: `#${filed.parent.issue.number}` })
                : filed.trigger?.name,
              filed.blockedBy.length > 0
                ? t("tasks.blockedBy", {
                    ids: filed.blockedBy
                      .map((blocker) => `#${blocker.issue?.number ?? ""}`)
                      .join(", "),
                  })
                : run
                  ? skillStep(run)
                  : null,
            ].filter(Boolean)
            return (
              <Row key={filed.id} cols="96px 1fr" tall>
                <TaskState status={filed.status} waitReason={filed.waitReason} />
                <div>
                  <Lk
                    to="/tasks/$task"
                    params={{ task: filed.id }}
                    className="font-medium no-underline"
                  >
                    {filed.title}
                  </Lk>{" "}
                  {filed.issue ? <Id>#{filed.issue.number}</Id> : null}
                  <Dim>{details.join(" · ")}</Dim>
                </div>
              </Row>
            )
          })}
        </Table>
      ) : null}
    </>
  )
}

export function ChatPage() {
  const params = useParams({ strict: false })
  const conversationId = params.conversation
  const { slug } = useTenant()
  const navigate = useNavigate()
  const { data, error, refetch } = useData(
    ChatDocument,
    { tenant: slug, conversation: conversationId ?? "", withConversation: !!conversationId },
    {
      poll: (result) => {
        const task = result.tenant?.conversation?.latestTask
        if (!conversationId) {
          return IDLE_POLL
        }
        return task && isActive(task.status) ? LIVE_POLL : IDLE_POLL
      },
    },
  )
  const [picked, setPicked] = useState<string>()
  const tenant = data?.tenant
  const conversation = tenant?.conversation
  const sessions = tenant?.sessions ?? []
  const [only] = sessions
  const session =
    sessions.length === 1 ? only : sessions.find((candidate) => candidate.id === picked)

  function newChat() {
    setPicked(undefined)
    void navigate({ to: "/chat" })
  }

  useKeys(conversationId ? { c: newChat } : {})

  return (
    <Page
      crumbs={[
        { label: t("nav.chat"), to: conversationId ? "/chat" : undefined },
        { label: conversationId ? (conversation?.title ?? "…") : t("chat.newChat") },
      ]}
      actions={
        conversationId ? (
          <Button variant="primary" icon={Plus} kbd="c" compact onPress={newChat}>
            {t("common.newChat")}
          </Button>
        ) : null
      }
      keys={conversationId ? [["c", "common.newChat"]] : []}
    >
      {error ? (
        <div className="px-4 py-6 md:px-6">
          <ErrorNote message={error.message} onRetry={refetch} />
        </div>
      ) : !tenant ? (
        <div className="px-4 py-6 md:px-6">
          <Skeleton rows={6} />
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-1 max-lg:content-start max-lg:overflow-auto lg:grid-cols-[260px_1fr_340px]">
          <div className="min-w-0 border-line px-4 py-[18px] max-lg:order-last max-lg:border-t md:px-5 lg:min-h-0 lg:overflow-auto">
            <Section count={tenant.conversations.length} first>
              {t("chat.threads")}
            </Section>
            {tenant.conversations.length === 0 ? <Dim>{t("chat.noThreads")}</Dim> : null}
            {tenant.conversations.map((item) => {
              const content = (
                <>
                  <span className="font-medium">{item.title}</span>
                  <small className="block text-xs text-muted">
                    {[
                      item.sessionName,
                      item.kind === "SLACK" ? t("chat.slack") : t("chat.web"),
                      t(item.filedTasks.length === 1 ? "chat.issueCountOne" : "chat.issuesCount", {
                        n: item.filedTasks.length,
                      }),
                    ].join(" · ")}
                  </small>
                </>
              )
              return item.id === conversationId ? (
                <div
                  key={item.id}
                  aria-current="page"
                  className="block rounded-ui bg-hover px-2 py-[7px]"
                >
                  {content}
                </div>
              ) : (
                <Link
                  key={item.id}
                  to="/chat/$conversation"
                  params={{ conversation: item.id }}
                  className="block rounded-ui px-2 py-[7px] hover:bg-hover"
                >
                  {content}
                </Link>
              )
            })}
          </div>
          <div className="min-w-0 border-line px-4 py-[18px] max-lg:order-first md:px-5 lg:min-h-0 lg:overflow-auto lg:border-l">
            {conversationId ? (
              conversation ? (
                <Thread conversation={conversation} tenant={tenant} />
              ) : (
                <Dim>{t("chat.notFound")}</Dim>
              )
            ) : (
              <NewChat tenant={tenant} session={session} onSelect={setPicked} />
            )}
          </div>
          <div className="min-w-0 border-line px-4 py-[18px] max-lg:border-t md:px-5 lg:min-h-0 lg:overflow-auto lg:border-l">
            {conversation ? (
              <FiledTasks conversation={conversation} />
            ) : (
              <Dim>{t("chat.filedHint")}</Dim>
            )}
          </div>
        </div>
      )}
    </Page>
  )
}
