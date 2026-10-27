import type { WorkerFactory } from "../worker.ts";
import { ClaudeWorker } from "./claude.ts";
import { CodexWorker } from "./codex.ts";

export const createWorker: WorkerFactory = (context) => {
  switch (context.agent) {
    case "claude":
      return new ClaudeWorker(context);
    case "codex":
      return new CodexWorker(context);
  }
};
