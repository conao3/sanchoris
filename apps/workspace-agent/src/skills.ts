import type { ErrorBody, InputType, SkillDefinition, SocketRequest } from "./protocol.ts"

function typeOf(value: unknown): InputType | "null" | "other" {
  if (value === null) return "null"
  if (Array.isArray(value)) return "array"
  switch (typeof value) {
    case "string":
      return "string"
    case "number":
      return "number"
    case "boolean":
      return "boolean"
    case "object":
      return "object"
    default:
      return "other"
  }
}

export function validateSkillRun(
  skills: readonly SkillDefinition[],
  request: Extract<SocketRequest, { action: "run" }>,
): ErrorBody | null {
  const skill = skills.find((candidate) => candidate.name === request.name)
  if (!skill) {
    const allowed = skills.map((candidate) => candidate.name).join(", ")
    return {
      code: "unknown_skill",
      message: `Skill ${request.name} is not available in this task. Available skills: ${allowed === "" ? "(none)" : allowed}`,
    }
  }
  if (typeOf(request.input) !== "object") {
    return { code: "invalid_input", message: "--input must be a JSON object" }
  }
  const input = request.input as Record<string, unknown>
  const problems: string[] = []
  for (const [field, expected] of Object.entries(skill.inputs)) {
    if (!(field in input)) {
      problems.push(`${field} is required (${expected})`)
      continue
    }
    const actual = typeOf(input[field])
    if (actual !== expected) problems.push(`${field} must be ${expected}, got ${actual}`)
  }
  for (const field of Object.keys(input)) {
    if (!(field in skill.inputs)) problems.push(`${field} is not an input of ${skill.name}`)
  }
  if (problems.length > 0) {
    return {
      code: "invalid_input",
      message: `Invalid input for skill ${skill.name}: ${problems.join("; ")}`,
    }
  }
  return null
}
