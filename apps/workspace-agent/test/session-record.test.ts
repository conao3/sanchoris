import { describe, expect, it } from "vite-plus/test"
import { redactTokens } from "../src/session-record.ts"

const redact = (text: string) => redactTokens(Buffer.from(text, "utf8")).toString("utf8")

describe("redactTokens", () => {
  it("redacts the known token formats inside the record", () => {
    const github = `ghs_${"a1B2".repeat(9)}`
    const pat = `github_pat_${"x_Y9".repeat(20)}`
    const other = "Z".repeat(36)
    expect(
      redact(
        `${github} then https://x-access-token:${pat}@github.com, gho_${other} ghu_${other} ghp_${other}`,
      ),
    ).toBe(
      "<redacted> then https://x-access-token:<redacted>@github.com, <redacted> <redacted> <redacted>",
    )
    expect(redact(`key=sk-ant-oat01-${"aB_-3".repeat(5)} openai sk-proj-${"c".repeat(20)}`)).toBe(
      "key=<redacted> openai <redacted>",
    )
    const aws = "Y34FZKBOKMUTVV7A"
    expect(redact(`AKIA${aws} ASIA${aws}, AKIA${aws}X`)).toBe(`<redacted> <redacted>, AKIA${aws}X`)
  })

  it("keeps text that only looks like a token prefix", () => {
    const plain = '{"text":"task-0123456789abcdefghijklmn, ghs_short, sk-learn, 東京 ghp_"}\n'
    expect(redact(plain)).toBe(plain)
  })
})
