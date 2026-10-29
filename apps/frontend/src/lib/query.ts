import type { OperationVariables, TypedDocumentNode } from '@apollo/client';
import { useQuery } from '@apollo/client/react';
import { useEffect, useRef, useState } from 'react';

export const LIVE_POLL = 5000;
export const IDLE_POLL = 30000;

function usePageVisible(): boolean {
  const [visible, setVisible] = useState(() => document.visibilityState === 'visible');
  useEffect(() => {
    const update = () => setVisible(document.visibilityState === 'visible');
    document.addEventListener('visibilitychange', update);
    return () => document.removeEventListener('visibilitychange', update);
  }, []);
  return visible;
}

export function useData<TData, TVariables extends OperationVariables>(
  document: TypedDocumentNode<TData, TVariables>,
  variables: TVariables,
  options: { skip?: boolean; poll?: (data: TData) => number } = {},
) {
  const visible = usePageVisible();
  const [polled, setPolled] = useState<TData>();
  const pollInterval = visible && polled && options.poll ? options.poll(polled) : 0;
  const result = useQuery(document, { variables, skip: options.skip, pollInterval, fetchPolicy: 'cache-and-network' } as useQuery.Options<TData, TVariables>);
  const data = result.dataState === 'complete' ? (result.data as TData) : undefined;
  const wasVisible = useRef(visible);
  const { refetch } = result;
  useEffect(() => {
    setPolled(data);
  }, [data]);
  useEffect(() => {
    if (visible && !wasVisible.current && pollInterval) {
      void refetch();
    }
    wasVisible.current = visible;
  }, [visible, pollInterval, refetch]);
  return {
    data,
    loading: result.loading,
    error: result.error,
    refetch: () => void result.refetch(),
  };
}
