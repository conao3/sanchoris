import { describe, expect, it } from "vitest";

import { parseArgs, UsageError } from "../src/cli.ts";
import { validateSkillRun } from "../src/skills.ts";

describe("parseArgs", () => {
  it("parses skill run with --input", () => {
    expect(parseArgs(["skill", "run", "open-pr-green-ci", "--input", '{"Issue":1}'])).toEqual({
      action: "run",
      name: "open-pr-green-ci",
      input: { Issue: 1 },
    });
  });

  it("parses --input=value and an omitted --input", () => {
    expect(parseArgs(["skill", "run", "a", "--input={}"])).toEqual({ action: "run", name: "a", input: {} });
    expect(parseArgs(["skill", "run", "a"])).toEqual({ action: "run", name: "a", input: {} });
  });

  it("parses continue and status", () => {
    expect(parseArgs(["skill", "continue", "3", "--output", '{"ok":true}'])).toEqual({
      action: "continue",
      skillRunId: 3,
      output: { ok: true },
    });
    expect(parseArgs(["skill", "continue", "3"])).toEqual({ action: "continue", skillRunId: 3, output: null });
    expect(parseArgs(["skill", "status", "12"])).toEqual({ action: "status", skillRunId: 12 });
  });

  it("rejects malformed invocations", () => {
    expect(() => parseArgs([])).toThrow(UsageError);
    expect(() => parseArgs(["skill", "run"])).toThrow("Missing skill name");
    expect(() => parseArgs(["skill", "run", "a", "--input", "{"])).toThrow("--input is not valid JSON");
    expect(() => parseArgs(["skill", "continue", "0"])).toThrow("Invalid skill run id");
    expect(() => parseArgs(["skill", "status", "1", "--output", "{}"])).toThrow("Unknown option");
    expect(() => parseArgs(["skill", "run", "a", "--input", "{}", "--input", "{}"])).toThrow("Duplicate option");
  });
});

describe("validateSkillRun", () => {
  const skills = [{ name: "s", required: false, inputs: { Repos: "array" as const, Issue: "number" as const } }];

  it("accepts matching input", () => {
    expect(validateSkillRun(skills, { action: "run", name: "s", input: { Repos: [], Issue: 1 } })).toBeNull();
  });

  it("rejects unknown skills, non-objects, missing, mistyped and extra fields", () => {
    expect(validateSkillRun(skills, { action: "run", name: "x", input: {} })?.code).toBe("unknown_skill");
    expect(validateSkillRun(skills, { action: "run", name: "s", input: [] })?.code).toBe("invalid_input");
    const error = validateSkillRun(skills, { action: "run", name: "s", input: { Repos: {}, Extra: 1 } });
    expect(error?.code).toBe("invalid_input");
    expect(error?.message).toContain("Repos must be array, got object");
    expect(error?.message).toContain("Issue is required");
    expect(error?.message).toContain("Extra is not an input of s");
  });
});
