import { useEffect, useRef, useState } from 'react';

export type KeyBindings = Record<string, () => void>;

const SEQUENCE_TIMEOUT = 1000;

function keyName(event: KeyboardEvent): string {
  const base = event.key === ' ' ? 'space' : event.key.length === 1 ? event.key.toLowerCase() : event.key;
  return event.metaKey || event.ctrlKey ? `mod+${base}` : base;
}

function inDialog(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest('[role="dialog"], [role="alertdialog"]') !== null;
}

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  return target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName);
}

export function useKeys(bindings: KeyBindings) {
  const bindingsRef = useRef(bindings);
  bindingsRef.current = bindings;

  useEffect(() => {
    let pending = '';
    let pendingAt = 0;

    function onKeyDown(event: KeyboardEvent) {
      if (event.altKey || event.isComposing || inDialog(event.target)) {
        return;
      }
      const name = keyName(event);
      const typing = isTyping(event.target);
      if (typing && !name.startsWith('mod+') && name !== 'Escape') {
        return;
      }
      const now = Date.now();
      const sequence = pending && now - pendingAt < SEQUENCE_TIMEOUT ? `${pending} ${name}` : name;
      const current = bindingsRef.current;
      const handler = current[sequence] ?? current[name];
      if (handler) {
        event.preventDefault();
        pending = '';
        handler();
        return;
      }
      const startsSequence = Object.keys(current).some((key) => key.startsWith(`${name} `));
      pending = startsSequence ? name : '';
      pendingAt = now;
    }

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);
}

export function useListNav(count: number, onOpen: (index: number) => void, extra: KeyBindings = {}) {
  const [index, setIndex] = useState(0);
  const rows = useRef<(HTMLDivElement | null)[]>([]);

  function move(next: number) {
    const clamped = Math.max(0, Math.min(count - 1, next));
    setIndex(clamped);
    rows.current[clamped]?.scrollIntoView({ block: 'nearest' });
  }

  useKeys({
    j: () => move(index + 1),
    k: () => move(index - 1),
    Enter: () => {
      if (count > 0) {
        onOpen(index);
      }
    },
    ...extra,
  });

  return {
    selected: Math.min(index, Math.max(0, count - 1)),
    select: setIndex,
    rowRef: (position: number) => (element: HTMLDivElement | null) => {
      rows.current[position] = element;
    },
  };
}
