import type { LimitPeriod } from '../graphql/generated/graphql';
import { t } from '../i18n';

export function planPrice(plan: { priceUsd: number }): string {
  return plan.priceUsd === 0 ? '$0' : t('plan.perMonth', { price: `$${plan.priceUsd}` });
}

export function planLimit(value: number, period: LimitPeriod): string {
  return period === 'TOTAL' ? t('plan.inTotal', { n: value }) : t('plan.perMonthCount', { n: value });
}
