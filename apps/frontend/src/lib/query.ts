import type { OperationVariables, TypedDocumentNode } from '@apollo/client';
import { useQuery } from '@apollo/client/react';

export function useData<TData, TVariables extends OperationVariables>(
  document: TypedDocumentNode<TData, TVariables>,
  variables: TVariables,
  options: { skip?: boolean; pollInterval?: number } = {},
) {
  const result = useQuery(document, { variables, skip: options.skip, pollInterval: options.pollInterval } as useQuery.Options<TData, TVariables>);
  return {
    data: result.dataState === 'complete' ? (result.data as TData) : undefined,
    loading: result.loading,
    error: result.error,
    refetch: () => void result.refetch(),
  };
}
