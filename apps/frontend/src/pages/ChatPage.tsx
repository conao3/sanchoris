import { Link, useNavigate, useParams } from "@tanstack/react-router"
import clsx from "clsx"
import { ChevronDown, CircleDot, Plus } from "lucide-react"
import { type KeyboardEvent, useState } from "react"
import { Button } from "../components/Button.tsx"
import { Menu } from "../components/Menu.tsx"
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
  type ChatQuery,
  ChatDocument,
  SendChatMessageDocument,
} from "../graphql/generated/graphql.ts"
import { t, tx } from "../i18n/index.ts"
import { clock } from "../lib/format.ts"
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
  disabled,
  onSend,
}: {
  disabled: boolean
  onSend: (text: string) => Promise<boolean>
}) {
  const [text, setText] = useState("")

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault()
      if (text.trim() && !disabled) {
        void onSend(text.trim()).then((sent) => {
          if (sent) {
            setText("")
          }
        })
      }
    }
  }

  return (
    <div className="mt-3.5 flex items-start justify-between gap-3 rounded-ui border border-line2 px-2.5 py-2">
      <textarea
        rows={1}
        value={text}
        disabled={disabled}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={onKeyDown}
        placeholder={t("chat.placeholder")}
        aria-label={t("chat.placeholder")}
        className="min-h-5 flex-1 resize-none bg-transparent outline-none placeholder:text-faint"
      />
      <Kbd>{t("chat.enterSend")}</Kbd>
    </div>
  )
}

function Thread({ conversation, tenant }: { conversation: ChatConversation; tenant: Tenant }) {
  const [send, { error }] = useAction(SendChatMessageDocument, { refetchQueries: [ChatDocument] })
  const task = conversation.latestTask
  const run = task?.runs[task.runs.length - 1]
  const repos = conversation.session?.repositories.join(", ") ?? ""
  const starter = conversation.createdBy?.user.name
  const finished = task ? !isActive(task.status) : false
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
      {conversation.messages.map((message) => (
        <div key={message.id} className="border-b border-line py-2.5">
          <div className="mb-0.5 text-sm font-semibold">
            {message.authorKind === "AGENT" ? "sanchoris" : (message.member?.user.name ?? "")}
            <span className="ml-1.5 font-normal text-faint">{clock(message.at)}</span>
          </div>
          <div className="whitespace-pre-wrap">{message.text}</div>
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
                  <Link to="/tasks/$task" params={{ task: issue.task.id }} className="font-medium">
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
        </div>
      ))}
      {finished && task ? (
        <Dim className="mt-2.5">
          {t("chat.continueHint", { n: (task.conversationSequence ?? 1) + 1 })}
        </Dim>
      ) : null}
      <Composer
        disabled={conversation.kind === "SLACK"}
        onSend={(text) =>
          send({
            input: { tenantId: tenant.id, conversationId: conversation.id, text },
          }).then(Boolean)
        }
      />
      <ActionError error={error} className="mt-2" />
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
          onClick={() => onSelect(session.id)}
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
        disabled={!session}
        onSend={(text) =>
          send({
            input: {
              tenantId: tenant.id,
              sessionName: session?.name,
              configId: session?.config.id,
              text,
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
  const [pickerOpen, setPickerOpen] = useState(false)
  const tenant = data?.tenant
  const conversation = tenant?.conversation
  const sessions = tenant?.sessions ?? []
  const [only] = sessions
  const session =
    sessions.length === 1 ? only : sessions.find((candidate) => candidate.id === picked)

  function newChat() {
    setPicked(undefined)
    setPickerOpen(sessions.length > 1)
    void navigate({ to: "/chat" })
  }

  function pick(id: string) {
    setPicked(id)
    if (conversationId) {
      void navigate({ to: "/chat" })
    }
  }

  useKeys({ c: newChat })

  return (
    <Page
      crumbs={[
        { label: t("nav.chat"), to: "/chat" },
        { label: conversationId ? (conversation?.title ?? "…") : t("chat.newChat") },
      ]}
      actions={
        <>
          <Button variant="primary" icon={Plus} kbd="c" compact onPress={newChat}>
            {t("common.newChat")}
          </Button>
          {sessions.length > 1 ? (
            <Menu
              icon={ChevronDown}
              label={t("chat.session")}
              title={t("chat.startIn")}
              align="right"
              compact
              open={pickerOpen}
              onOpenChange={setPickerOpen}
              value={session?.id}
              options={sessions.map((candidate) => ({
                value: candidate.id,
                label: candidate.name,
                detail: sessionDetail(candidate),
              }))}
              onChange={pick}
            />
          ) : null}
        </>
      }
      keys={[
        ["↵", "chat.send"],
        ["⇧↵", "chat.newLine"],
      ]}
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
            {tenant.conversations.map((item) => (
              <Link
                key={item.id}
                to="/chat/$conversation"
                params={{ conversation: item.id }}
                className={clsx(
                  "block rounded-ui px-2 py-[7px] hover:bg-hover",
                  item.id === conversationId && "bg-hover",
                )}
              >
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
              </Link>
            ))}
          </div>
          <div className="min-w-0 border-line px-4 py-[18px] max-lg:order-first md:px-5 lg:min-h-0 lg:overflow-auto lg:border-l">
            {conversationId ? (
              conversation ? (
                <Thread conversation={conversation} tenant={tenant} />
              ) : (
                <Dim>{t("chat.notFound")}</Dim>
              )
            ) : (
              <NewChat tenant={tenant} session={session} onSelect={pick} />
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
