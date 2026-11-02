import { Check } from "lucide-react"
import { type FormEvent, useState } from "react"
import { type ApiKeyProvider, AddCredentialDocument } from "../graphql/generated/graphql.ts"
import { t } from "../i18n/index.ts"
import { useAction } from "../lib/mutation.ts"
import { Button } from "./Button.tsx"
import { Input, Label, Select } from "./Field.tsx"
import { ActionError, Code } from "./ui.tsx"

export function SubscriptionCommands({ tenantName }: { tenantName: string }) {
  return (
    <Code>
      <span className="text-faint"># {t("setup.claudeSubscription")}</span>
      {"\n$ sanchoris claude-auth\n"}
      <span className="text-faint"># {t("setup.codexSubscription")}</span>
      {"\n$ sanchoris codex-auth\n"}
      <span className="text-faint"># {t("setup.shareWithTenant", { tenant: tenantName })}</span>
      {"\n$ sanchoris claude-auth --org"}
    </Code>
  )
}

export function ApiKeyForm({ tenantId, reason }: { tenantId: string; reason?: string }) {
  const [provider, setProvider] = useState<ApiKeyProvider>("ANTHROPIC")
  const [name, setName] = useState("")
  const [key, setKey] = useState("")
  const [addCredential, { loading, error }] = useAction(AddCredentialDocument, {
    refetchQueries: "active",
  })

  function submit(event: FormEvent) {
    event.preventDefault()
    void addCredential({ input: { tenantId, name, provider, key, scope: "ORGANIZATION" } }).then(
      (done) => {
        if (!done) {
          return
        }
        setName("")
        setKey("")
      },
    )
  }

  return (
    <form onSubmit={submit}>
      <fieldset disabled={!!reason} className="contents">
        <Label htmlFor="provider">{t("setup.provider")}</Label>
        <Select
          id="provider"
          className="w-full"
          value={provider}
          onChange={(event) => setProvider(event.target.value as ApiKeyProvider)}
        >
          <option value="ANTHROPIC">{t("setup.anthropicKey")}</option>
          <option value="OPENAI">{t("setup.openaiKey")}</option>
          <option value="BEDROCK">{t("setup.bedrock")}</option>
        </Select>
        <Label htmlFor="credential-name">{t("setup.name")}</Label>
        <Input
          id="credential-name"
          mono
          className="w-full"
          placeholder={t("placeholder.credentialName")}
          value={name}
          onChange={(event) => setName(event.target.value)}
          required
        />
        <Label htmlFor="credential-key">{t("setup.key")}</Label>
        <Input
          id="credential-key"
          type="password"
          mono
          className="w-full"
          placeholder={t("placeholder.apiKey")}
          value={key}
          onChange={(event) => setKey(event.target.value)}
          required
        />
        <ActionError error={error} className="mt-2" />
        <div className="mt-3.5">
          <Button type="submit" variant="primary" icon={Check} isDisabled={loading} reason={reason}>
            {t("setup.saveKey")}
          </Button>
        </div>
      </fieldset>
    </form>
  )
}
