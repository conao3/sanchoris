import { Fragment, type ReactNode, createElement } from 'react';
import en from './en.json';
import ja from './ja.json';

export type MessageKey = keyof typeof en;
type Params = Record<string, string | number>;

const catalogs: Record<'en' | 'ja', Record<MessageKey, string>> = { en, ja };

export const locale = navigator.language.toLowerCase().startsWith('ja') ? 'ja' : 'en';

const messages = catalogs[locale];

export function t(key: MessageKey, params?: Params): string {
  const text = messages[key];
  if (!params) {
    return text;
  }
  return text.replace(/\{(\w+)\}/g, (match, name: string) => (name in params ? String(params[name]) : match));
}

export function tx(key: MessageKey, parts: Record<string, ReactNode>): ReactNode {
  const pieces = messages[key].split(/(\{\w+\})/);
  return createElement(
    Fragment,
    null,
    ...pieces.map((piece, index) => {
      const name = /^\{(\w+)\}$/.exec(piece)?.[1];
      return createElement(Fragment, { key: index }, name && name in parts ? parts[name] : piece);
    }),
  );
}
