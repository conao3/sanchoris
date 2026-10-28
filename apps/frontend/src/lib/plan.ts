import type { LimitPeriod } from '../graphql/generated/graphql';
import { t } from '../i18n';

export function planPrice(plan: { priceUsd: number }): string {
  return plan.priceUsd === 0 ? '$0' : t('plan.perMonth', { price: `$${plan.priceUsd}` });
}

export function planLimit(value: number | null | undefined, period: LimitPeriod): string {
  if (value == null) {
    return t('plan.noLimit');
  }
  return period === 'TOTAL' ? t('plan.inTotal', { n: value }) : t('plan.perMonthCount', { n: value });
}

export function countOf(n: number, total: number | null | undefined): string {
  return total == null ? t('plan.countNoLimit', { n }) : t('home.countOf', { n, total });
}

export function hoursOf(n: number, total: number | null | undefined): string {
  return total == null ? t('plan.hoursNoLimit', { n }) : t('home.hoursOf', { n, total });
}

export function usedShare(used: number, limit: number | null | undefined): number {
  return limit == null ? 0 : used / limit;
}
