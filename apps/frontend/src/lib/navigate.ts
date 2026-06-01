import { Link } from '@tanstack/react-router';
import type { AnyRouter } from '@tanstack/react-router';

export { Link };

let _router: AnyRouter | null = null;

export function setRouter(r: AnyRouter): void {
  _router = r;
}

export function navigate(path: string): void {
  if (_router) {
    void _router.navigate({ to: path });
  } else {
    window.location.href = path;
  }
}
