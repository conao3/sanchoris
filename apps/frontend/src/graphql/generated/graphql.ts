/* eslint-disable */
/** Internal type. DO NOT USE DIRECTLY. */
type Exact<T extends { [key: string]: unknown }> = { [K in keyof T]: T[K] };
/** Internal type. DO NOT USE DIRECTLY. */
export type Incremental<T> = T | { [P in keyof T]?: P extends ' $fragmentName' | '__typename' ? T[P] : never };
import { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';
export type CreateTaskInput = {
  conversationId: string;
  description: string;
  priority?: Priority | null | undefined;
  projectId: string;
  title: string;
  worker?: string | null | undefined;
  workflowId: string;
};

export type Priority =
  | 'HIGH'
  | 'LOW'
  | 'MEDIUM';

export type RunStatus =
  | 'FAILED'
  | 'GATE_PENDING'
  | 'PASSED'
  | 'QUEUED'
  | 'RUNNING';

export type StepState =
  | 'BLOCKED'
  | 'FAILED'
  | 'PASSED'
  | 'PENDING'
  | 'RUNNING';

export type TaskStatus =
  | 'DONE'
  | 'FAILED'
  | 'QUEUED'
  | 'REVIEW'
  | 'RUNNING';

export type MvpShellQueryVariables = Exact<{ [key: string]: never; }>;


export type MvpShellQuery = { viewer: { id: string, displayName: string }, projectProfiles: Array<{ id: string, name: string, repositoryPath: string, defaultBranch: string, worktreePolicy: string, devCommand: string, checkCommand: string, workerPolicy: string, allowedWorkflowId: string }>, conversations: Array<{ id: string, title: string, channel: string, messages: Array<{ id: string, author: string, body: string, createdAt: string, taskId: string | null }> }>, tasks: Array<{ id: string, title: string, description: string, status: TaskStatus, priority: Priority, conversationId: string, assignedWorkflowId: string, assignedWorker: string, workspaceId: string, latestRunId: string, reviewState: string }>, workflowSpecs: Array<{ id: string, name: string, version: string, yaml: string, blocks: Array<{ id: string, kind: string, label: string, state: StepState, x: number, y: number }>, edges: Array<{ from: string, to: string }> }>, runs: Array<{ id: string, taskId: string, workerKind: string, status: RunStatus, startedAt: string, finishedAt: string | null, exitCode: number | null, commitHash: string | null, logUri: string, errorSummary: string | null, workspace: { id: string, branch: string, worktreePath: string, baseCommit: string, currentCommit: string | null, cleanupState: string }, verification: { command: string, exitCode: number | null, status: StepState, summary: string, artifactUri: string }, pullRequest: { url: string | null, sourceBranch: string, baseBranch: string, status: string }, merge: { method: string, status: string, mergeCommit: string | null, mergedAt: string | null, mergedBy: string | null, failureReason: string | null }, gate: { id: string, state: string, reviewTarget: string, approver: string | null, decidedAt: string | null } }> };

export type CreateTaskMutationVariables = Exact<{
  input: CreateTaskInput;
}>;


export type CreateTaskMutation = { createTask: { id: string, title: string, status: TaskStatus, priority: Priority, assignedWorker: string } };

export type ValidateWorkflowCanvasMutationVariables = Exact<{
  workflowId: string;
}>;


export type ValidateWorkflowCanvasMutation = { validateWorkflowCanvas: { workflowId: string, valid: boolean, errors: Array<string> } };


export const MvpShellDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"MvpShell"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"viewer"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"displayName"}}]}},{"kind":"Field","name":{"kind":"Name","value":"projectProfiles"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"repositoryPath"}},{"kind":"Field","name":{"kind":"Name","value":"defaultBranch"}},{"kind":"Field","name":{"kind":"Name","value":"worktreePolicy"}},{"kind":"Field","name":{"kind":"Name","value":"devCommand"}},{"kind":"Field","name":{"kind":"Name","value":"checkCommand"}},{"kind":"Field","name":{"kind":"Name","value":"workerPolicy"}},{"kind":"Field","name":{"kind":"Name","value":"allowedWorkflowId"}}]}},{"kind":"Field","name":{"kind":"Name","value":"conversations"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"channel"}},{"kind":"Field","name":{"kind":"Name","value":"messages"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"author"}},{"kind":"Field","name":{"kind":"Name","value":"body"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"taskId"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"tasks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"priority"}},{"kind":"Field","name":{"kind":"Name","value":"conversationId"}},{"kind":"Field","name":{"kind":"Name","value":"assignedWorkflowId"}},{"kind":"Field","name":{"kind":"Name","value":"assignedWorker"}},{"kind":"Field","name":{"kind":"Name","value":"workspaceId"}},{"kind":"Field","name":{"kind":"Name","value":"latestRunId"}},{"kind":"Field","name":{"kind":"Name","value":"reviewState"}}]}},{"kind":"Field","name":{"kind":"Name","value":"workflowSpecs"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"version"}},{"kind":"Field","name":{"kind":"Name","value":"yaml"}},{"kind":"Field","name":{"kind":"Name","value":"blocks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"state"}},{"kind":"Field","name":{"kind":"Name","value":"x"}},{"kind":"Field","name":{"kind":"Name","value":"y"}}]}},{"kind":"Field","name":{"kind":"Name","value":"edges"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"from"}},{"kind":"Field","name":{"kind":"Name","value":"to"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"runs"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"taskId"}},{"kind":"Field","name":{"kind":"Name","value":"workerKind"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"finishedAt"}},{"kind":"Field","name":{"kind":"Name","value":"exitCode"}},{"kind":"Field","name":{"kind":"Name","value":"commitHash"}},{"kind":"Field","name":{"kind":"Name","value":"logUri"}},{"kind":"Field","name":{"kind":"Name","value":"errorSummary"}},{"kind":"Field","name":{"kind":"Name","value":"workspace"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"branch"}},{"kind":"Field","name":{"kind":"Name","value":"worktreePath"}},{"kind":"Field","name":{"kind":"Name","value":"baseCommit"}},{"kind":"Field","name":{"kind":"Name","value":"currentCommit"}},{"kind":"Field","name":{"kind":"Name","value":"cleanupState"}}]}},{"kind":"Field","name":{"kind":"Name","value":"verification"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"command"}},{"kind":"Field","name":{"kind":"Name","value":"exitCode"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"summary"}},{"kind":"Field","name":{"kind":"Name","value":"artifactUri"}}]}},{"kind":"Field","name":{"kind":"Name","value":"pullRequest"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"url"}},{"kind":"Field","name":{"kind":"Name","value":"sourceBranch"}},{"kind":"Field","name":{"kind":"Name","value":"baseBranch"}},{"kind":"Field","name":{"kind":"Name","value":"status"}}]}},{"kind":"Field","name":{"kind":"Name","value":"merge"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"method"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"mergeCommit"}},{"kind":"Field","name":{"kind":"Name","value":"mergedAt"}},{"kind":"Field","name":{"kind":"Name","value":"mergedBy"}},{"kind":"Field","name":{"kind":"Name","value":"failureReason"}}]}},{"kind":"Field","name":{"kind":"Name","value":"gate"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"state"}},{"kind":"Field","name":{"kind":"Name","value":"reviewTarget"}},{"kind":"Field","name":{"kind":"Name","value":"approver"}},{"kind":"Field","name":{"kind":"Name","value":"decidedAt"}}]}}]}}]}}]} as unknown as DocumentNode<MvpShellQuery, MvpShellQueryVariables>;
export const CreateTaskDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"CreateTask"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"CreateTaskInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"createTask"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"priority"}},{"kind":"Field","name":{"kind":"Name","value":"assignedWorker"}}]}}]}}]} as unknown as DocumentNode<CreateTaskMutation, CreateTaskMutationVariables>;
export const ValidateWorkflowCanvasDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ValidateWorkflowCanvas"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"workflowId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"validateWorkflowCanvas"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"workflowId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"workflowId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"workflowId"}},{"kind":"Field","name":{"kind":"Name","value":"valid"}},{"kind":"Field","name":{"kind":"Name","value":"errors"}}]}}]}}]} as unknown as DocumentNode<ValidateWorkflowCanvasMutation, ValidateWorkflowCanvasMutationVariables>;