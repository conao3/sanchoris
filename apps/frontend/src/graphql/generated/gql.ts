/* eslint-disable */
import * as types from './graphql';
import { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';

/**
 * Map of all GraphQL operations in the project.
 *
 * This map has several performance disadvantages:
 * 1. It is not tree-shakeable, so it will include all operations in the project.
 * 2. It is not minifiable, so the string of a GraphQL query will be multiple times inside the bundle.
 * 3. It does not support dead code elimination, so it will add unused operations.
 *
 * Therefore it is highly recommended to use the babel or swc plugin for production.
 * Learn more about it here: https://the-guild.dev/graphql/codegen/plugins/presets/preset-client#reducing-bundle-size
 */
type Documents = {
    "query MvpShell {\n  viewer {\n    id\n    displayName\n  }\n  projectProfiles {\n    id\n    name\n    repositoryPath\n    defaultBranch\n    worktreePolicy\n    devCommand\n    checkCommand\n    workerPolicy\n    allowedWorkflowId\n  }\n  conversations {\n    id\n    title\n    channel\n    messages {\n      id\n      author\n      body\n      createdAt\n      taskId\n    }\n  }\n  tasks {\n    id\n    title\n    description\n    status\n    priority\n    conversationId\n    assignedWorkflowId\n    assignedWorker\n    workspaceId\n    latestRunId\n    reviewState\n  }\n  workflowSpecs {\n    id\n    name\n    version\n    yaml\n    blocks {\n      id\n      kind\n      label\n      state\n      x\n      y\n    }\n    edges {\n      from\n      to\n    }\n  }\n  runs {\n    id\n    taskId\n    workerKind\n    status\n    startedAt\n    finishedAt\n    exitCode\n    commitHash\n    logUri\n    errorSummary\n    workspace {\n      id\n      branch\n      worktreePath\n      baseCommit\n      currentCommit\n      cleanupState\n    }\n    verification {\n      command\n      exitCode\n      status\n      summary\n      artifactUri\n    }\n    pullRequest {\n      url\n      sourceBranch\n      baseBranch\n      status\n    }\n    merge {\n      method\n      status\n      mergeCommit\n      mergedAt\n      mergedBy\n      failureReason\n    }\n    gate {\n      id\n      state\n      reviewTarget\n      approver\n      decidedAt\n    }\n  }\n}\n\nmutation CreateTask($input: CreateTaskInput!) {\n  createTask(input: $input) {\n    id\n    title\n    status\n    priority\n    assignedWorker\n  }\n}\n\nmutation ValidateWorkflowCanvas($workflowId: String!) {\n  validateWorkflowCanvas(workflowId: $workflowId) {\n    workflowId\n    valid\n    errors\n  }\n}": typeof types.MvpShellDocument,
};
const documents: Documents = {
    "query MvpShell {\n  viewer {\n    id\n    displayName\n  }\n  projectProfiles {\n    id\n    name\n    repositoryPath\n    defaultBranch\n    worktreePolicy\n    devCommand\n    checkCommand\n    workerPolicy\n    allowedWorkflowId\n  }\n  conversations {\n    id\n    title\n    channel\n    messages {\n      id\n      author\n      body\n      createdAt\n      taskId\n    }\n  }\n  tasks {\n    id\n    title\n    description\n    status\n    priority\n    conversationId\n    assignedWorkflowId\n    assignedWorker\n    workspaceId\n    latestRunId\n    reviewState\n  }\n  workflowSpecs {\n    id\n    name\n    version\n    yaml\n    blocks {\n      id\n      kind\n      label\n      state\n      x\n      y\n    }\n    edges {\n      from\n      to\n    }\n  }\n  runs {\n    id\n    taskId\n    workerKind\n    status\n    startedAt\n    finishedAt\n    exitCode\n    commitHash\n    logUri\n    errorSummary\n    workspace {\n      id\n      branch\n      worktreePath\n      baseCommit\n      currentCommit\n      cleanupState\n    }\n    verification {\n      command\n      exitCode\n      status\n      summary\n      artifactUri\n    }\n    pullRequest {\n      url\n      sourceBranch\n      baseBranch\n      status\n    }\n    merge {\n      method\n      status\n      mergeCommit\n      mergedAt\n      mergedBy\n      failureReason\n    }\n    gate {\n      id\n      state\n      reviewTarget\n      approver\n      decidedAt\n    }\n  }\n}\n\nmutation CreateTask($input: CreateTaskInput!) {\n  createTask(input: $input) {\n    id\n    title\n    status\n    priority\n    assignedWorker\n  }\n}\n\nmutation ValidateWorkflowCanvas($workflowId: String!) {\n  validateWorkflowCanvas(workflowId: $workflowId) {\n    workflowId\n    valid\n    errors\n  }\n}": types.MvpShellDocument,
};

/**
 * The graphql function is used to parse GraphQL queries into a document that can be used by GraphQL clients.
 *
 *
 * @example
 * ```ts
 * const query = graphql(`query GetUser($id: ID!) { user(id: $id) { name } }`);
 * ```
 *
 * The query argument is unknown!
 * Please regenerate the types.
 */
export function graphql(source: string): unknown;

/**
 * The graphql function is used to parse GraphQL queries into a document that can be used by GraphQL clients.
 */
export function graphql(source: "query MvpShell {\n  viewer {\n    id\n    displayName\n  }\n  projectProfiles {\n    id\n    name\n    repositoryPath\n    defaultBranch\n    worktreePolicy\n    devCommand\n    checkCommand\n    workerPolicy\n    allowedWorkflowId\n  }\n  conversations {\n    id\n    title\n    channel\n    messages {\n      id\n      author\n      body\n      createdAt\n      taskId\n    }\n  }\n  tasks {\n    id\n    title\n    description\n    status\n    priority\n    conversationId\n    assignedWorkflowId\n    assignedWorker\n    workspaceId\n    latestRunId\n    reviewState\n  }\n  workflowSpecs {\n    id\n    name\n    version\n    yaml\n    blocks {\n      id\n      kind\n      label\n      state\n      x\n      y\n    }\n    edges {\n      from\n      to\n    }\n  }\n  runs {\n    id\n    taskId\n    workerKind\n    status\n    startedAt\n    finishedAt\n    exitCode\n    commitHash\n    logUri\n    errorSummary\n    workspace {\n      id\n      branch\n      worktreePath\n      baseCommit\n      currentCommit\n      cleanupState\n    }\n    verification {\n      command\n      exitCode\n      status\n      summary\n      artifactUri\n    }\n    pullRequest {\n      url\n      sourceBranch\n      baseBranch\n      status\n    }\n    merge {\n      method\n      status\n      mergeCommit\n      mergedAt\n      mergedBy\n      failureReason\n    }\n    gate {\n      id\n      state\n      reviewTarget\n      approver\n      decidedAt\n    }\n  }\n}\n\nmutation CreateTask($input: CreateTaskInput!) {\n  createTask(input: $input) {\n    id\n    title\n    status\n    priority\n    assignedWorker\n  }\n}\n\nmutation ValidateWorkflowCanvas($workflowId: String!) {\n  validateWorkflowCanvas(workflowId: $workflowId) {\n    workflowId\n    valid\n    errors\n  }\n}"): (typeof documents)["query MvpShell {\n  viewer {\n    id\n    displayName\n  }\n  projectProfiles {\n    id\n    name\n    repositoryPath\n    defaultBranch\n    worktreePolicy\n    devCommand\n    checkCommand\n    workerPolicy\n    allowedWorkflowId\n  }\n  conversations {\n    id\n    title\n    channel\n    messages {\n      id\n      author\n      body\n      createdAt\n      taskId\n    }\n  }\n  tasks {\n    id\n    title\n    description\n    status\n    priority\n    conversationId\n    assignedWorkflowId\n    assignedWorker\n    workspaceId\n    latestRunId\n    reviewState\n  }\n  workflowSpecs {\n    id\n    name\n    version\n    yaml\n    blocks {\n      id\n      kind\n      label\n      state\n      x\n      y\n    }\n    edges {\n      from\n      to\n    }\n  }\n  runs {\n    id\n    taskId\n    workerKind\n    status\n    startedAt\n    finishedAt\n    exitCode\n    commitHash\n    logUri\n    errorSummary\n    workspace {\n      id\n      branch\n      worktreePath\n      baseCommit\n      currentCommit\n      cleanupState\n    }\n    verification {\n      command\n      exitCode\n      status\n      summary\n      artifactUri\n    }\n    pullRequest {\n      url\n      sourceBranch\n      baseBranch\n      status\n    }\n    merge {\n      method\n      status\n      mergeCommit\n      mergedAt\n      mergedBy\n      failureReason\n    }\n    gate {\n      id\n      state\n      reviewTarget\n      approver\n      decidedAt\n    }\n  }\n}\n\nmutation CreateTask($input: CreateTaskInput!) {\n  createTask(input: $input) {\n    id\n    title\n    status\n    priority\n    assignedWorker\n  }\n}\n\nmutation ValidateWorkflowCanvas($workflowId: String!) {\n  validateWorkflowCanvas(workflowId: $workflowId) {\n    workflowId\n    valid\n    errors\n  }\n}"];

export function graphql(source: string) {
  return (documents as any)[source] ?? {};
}

export type DocumentType<TDocumentNode extends DocumentNode<any, any>> = TDocumentNode extends DocumentNode<  infer TType,  any>  ? TType  : never;