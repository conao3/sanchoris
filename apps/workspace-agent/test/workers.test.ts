import { describe, expect, it } from "vite-plus/test"

import {
  claudeAnswers,
  claudeApproval,
  claudePermissionMode,
  claudeQuestions,
  claudeTodoSteps,
} from "../src/workers/claude.ts"
import { codexApproval, codexPlanSteps, codexQuestions } from "../src/workers/codex.ts"

describe("Claude AskUserQuestion", () => {
  it("names each question by its text and joins the picked labels for the SDK", () => {
    const questions = claudeQuestions({
      questions: [
        {
          question: "Which checks?",
          header: "Checks",
          multiSelect: true,
          options: [
            { label: "Unit", description: "pnpm test" },
            { label: "Lint", description: "pnpm lint" },
          ],
        },
        { question: "Ship it?", header: "Ship", options: [{ label: "Yes" }] },
      ],
    })
    expect(questions).toEqual([
      {
        id: "Which checks?",
        header: "Checks",
        question: "Which checks?",
        multiSelect: true,
        options: [
          { label: "Unit", description: "pnpm test" },
          { label: "Lint", description: "pnpm lint" },
        ],
      },
      {
        id: "Ship it?",
        header: "Ship",
        question: "Ship it?",
        multiSelect: false,
        options: [{ label: "Yes", description: "" }],
      },
    ])
    expect(
      claudeAnswers(questions, { "Which checks?": ["Unit", "Lint"], "Ship it?": ["Yes"] }),
    ).toEqual({ "Which checks?": "Unit, Lint", "Ship it?": "Yes" })
  })

  it("rejects a call without questions", () => {
    expect(() => claudeQuestions({ questions: [] })).toThrow("AskUserQuestion has no questions")
  })
})

describe("Codex requestUserInput", () => {
  it("keeps the ids, headers and options of the questions", () => {
    expect(
      codexQuestions({
        threadId: "t",
        turnId: "u",
        itemId: "i",
        questions: [
          {
            id: "env",
            header: "Environment",
            question: "Where should I deploy?",
            isOther: true,
            options: [{ label: "dev", description: "The dev stack" }],
          },
          { id: "note", header: "Note", question: "Anything else?", options: null },
        ],
      }),
    ).toEqual([
      {
        id: "env",
        header: "Environment",
        question: "Where should I deploy?",
        multiSelect: false,
        options: [{ label: "dev", description: "The dev stack" }],
      },
      { id: "note", header: "Note", question: "Anything else?", multiSelect: false, options: [] },
    ])
  })
})

describe("session modes", () => {
  it("maps the chat modes to Claude permission modes, plan mode first", () => {
    expect(claudePermissionMode({ interactionMode: "plan", runtimeMode: "full-access" })).toBe(
      "plan",
    )
    expect(
      claudePermissionMode({ interactionMode: "default", runtimeMode: "approval-required" }),
    ).toBe("default")
    expect(
      claudePermissionMode({ interactionMode: "default", runtimeMode: "auto-accept-edits" }),
    ).toBe("acceptEdits")
    expect(claudePermissionMode({ interactionMode: "default", runtimeMode: "full-access" })).toBe(
      "bypassPermissions",
    )
  })
})

describe("plans", () => {
  it("reads Claude TodoWrite and Codex plan updates as steps", () => {
    expect(
      claudeTodoSteps({
        todos: [
          { content: "Read the code", status: "completed", activeForm: "Reading" },
          { content: "Fix it", status: "in_progress", activeForm: "Fixing" },
          { content: "", status: "pending" },
        ],
      }),
    ).toEqual([
      { text: "Read the code", status: "completed" },
      { text: "Fix it", status: "running" },
    ])
    expect(
      codexPlanSteps({
        plan: [
          { step: "Plan", status: "completed" },
          { step: " ", status: "inProgress" },
        ],
      }),
    ).toEqual([
      { text: "Plan", status: "completed" },
      { text: "Step 2", status: "running" },
    ])
  })
})

describe("approvals", () => {
  it("describes Claude and Codex approval requests by kind and detail", () => {
    expect(claudeApproval("Bash", { command: "rm -rf build" }, "Deletes files")).toEqual({
      kind: "command",
      toolName: "Bash",
      detail: "rm -rf build",
      reason: "Deletes files",
      input: { command: "rm -rf build" },
    })
    expect(claudeApproval("Edit", { file_path: "/workspace/a.ts" }, undefined).kind).toBe(
      "file-change",
    )
    expect(claudeApproval("WebFetch", { url: "https://example.com" }, undefined).detail).toBe(
      '{"url":"https://example.com"}',
    )
    expect(
      codexApproval("item/commandExecution/requestApproval", {
        command: "pnpm install",
        cwd: "/workspace",
        reason: "Needs network",
      }),
    ).toEqual({
      kind: "command",
      toolName: "commandExecution",
      detail: "pnpm install",
      reason: "Needs network",
      input: { command: "pnpm install", cwd: "/workspace" },
    })
    expect(codexApproval("item/fileChange/requestApproval", { grantRoot: "/workspace" })).toEqual({
      kind: "file-change",
      toolName: "fileChange",
      detail: "/workspace",
      reason: null,
      input: { grantRoot: "/workspace" },
    })
  })
})
