import { locale, t } from '../i18n';

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

const clockFormat = new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
const weekdayFormat = new Intl.DateTimeFormat(locale, { weekday: 'short' });
const dayFormat = new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric' });
const dayTimeFormat = new Intl.DateTimeFormat(locale, {
  month: 'short',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});
const weekdayTimeFormat = new Intl.DateTimeFormat(locale, {
  weekday: 'short',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});
const usdFormat = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' });
const percentFormat = new Intl.NumberFormat(locale, { style: 'percent', maximumFractionDigits: 0 });

function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

export function usd(value: number): string {
  return usdFormat.format(value);
}

export function usdWhole(value: number): string {
  return `$${value}`;
}

export function percent(ratio: number): string {
  return percentFormat.format(ratio);
}

export function clock(iso: string): string {
  return clockFormat.format(new Date(iso));
}

export function day(iso: string): string {
  return dayFormat.format(new Date(iso));
}

export function dayTime(iso: string): string {
  return dayTimeFormat.format(new Date(iso));
}

export function weekdayTime(iso: string): string {
  return weekdayTimeFormat.format(new Date(iso));
}

export function when(iso: string): string {
  const date = new Date(iso);
  const today = startOfDay(new Date());
  if (startOfDay(date) === today) {
    return clockFormat.format(date);
  }
  if (today - startOfDay(date) < 7 * DAY) {
    return weekdayFormat.format(date);
  }
  return dayFormat.format(date);
}

export function age(iso: string): string {
  const elapsed = Date.now() - new Date(iso).getTime();
  if (elapsed < MINUTE) {
    return t('time.now');
  }
  if (elapsed < HOUR) {
    return t('time.minutes', { n: Math.floor(elapsed / MINUTE) });
  }
  if (elapsed < DAY) {
    return t('time.hours', { n: Math.floor(elapsed / HOUR) });
  }
  return t('time.days', { n: Math.floor(elapsed / DAY) });
}

export function ago(iso: string): string {
  const elapsed = Date.now() - new Date(iso).getTime();
  if (elapsed < MINUTE) {
    return t('time.now');
  }
  if (elapsed >= DAY && startOfDay(new Date(iso)) === startOfDay(new Date()) - DAY) {
    return t('time.yesterday');
  }
  return t('time.ago', { age: age(iso) });
}

export function until(iso: string): string {
  const remaining = new Date(iso).getTime() - Date.now();
  return t('time.inDays', { n: Math.max(0, Math.round(remaining / DAY)) });
}

export function duration(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.round((seconds % 3600) / 60);
  if (hours > 0) {
    return t('duration.hm', { h: hours, m: minutes });
  }
  return t('duration.m', { m: Math.max(1, minutes) });
}

export function durationLong(seconds: number): string {
  return t('duration.minutesLong', { m: Math.round(seconds / 60) });
}

export function between(startIso: string, endIso: string): number {
  return Math.round((new Date(endIso).getTime() - new Date(startIso).getTime()) / 1000);
}

export function hours(seconds: number): number {
  return Math.round(seconds / 3600);
}

export function elapsedRatio(startIso: string, endIso: string): number {
  const start = new Date(startIso).getTime();
  const end = new Date(endIso).getTime();
  return Math.min(1, Math.max(0, (Date.now() - start) / (end - start)));
}

export function issueLabel(repository: string, number: number): string {
  return `${repository.split('/').pop()}#${number}`;
}
