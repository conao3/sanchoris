import type { OperationVariables, TypedDocumentNode } from "@apollo/client"
import { useMutation } from "@apollo/client/react"

export function useAction<TData, TVariables extends OperationVariables>(
  document: TypedDocumentNode<TData, TVariables>,
  options?: useMutation.Options<TData, TVariables>,
) {
  const [mutate, state] = useMutation<TData, TVariables>(document, options)
  function run(variables: TVariables): Promise<TData | undefined> {
    return mutate(...([{ variables }] as Parameters<typeof mutate>)).then(
      (result) => result.data ?? undefined,
      () => undefined,
    )
  }
  return [run, state] as const
}
