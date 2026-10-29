import { useNavigate } from '@tanstack/react-router';
import { Ban, CircleCheck, CircleX, Clock } from 'lucide-react';
import { useEffect, useState } from 'react';
import type { TenantState } from '../graphql/generated/graphql';
import { type MessageKey, t } from '../i18n';
import { State } from './ui';

export type CheckoutOutcome = 'success' | 'canceled';

export function checkoutSearch(search: Record<string, unknown>): { checkout?: CheckoutOutcome } {
  return search.checkout === 'success' || search.checkout === 'canceled' ? { checkout: search.checkout } : {};
}

export function useCheckoutOutcome(checkout: CheckoutOutcome | undefined): CheckoutOutcome | undefined {
  const navigate = useNavigate();
  const [returned] = useState(checkout);
  useEffect(() => {
    if (checkout) {
      void navigate({ to: '.', search: {}, replace: true });
    }
  }, [checkout, navigate]);
  return returned;
}

export function CheckoutNote({ outcome, success, canceled }: { outcome?: CheckoutOutcome; success: MessageKey; canceled: MessageKey }) {
  if (!outcome) {
    return null;
  }
  return (
    <div className="mb-3.5">
      {outcome === 'success' ? (
        <State tone="ok" icon={CircleCheck} wrap>
          {t(success)}
        </State>
      ) : (
        <State tone="wait" icon={CircleX} wrap>
          {t(canceled)}
        </State>
      )}
    </div>
  );
}

const reasons: Record<string, MessageKey> = {
  payment_failed: 'tenantState.paymentFailed',
};

export function TenantStateNote({ tenant }: { tenant: { state: TenantState; suspendedReason?: string | null } }) {
  if (tenant.state !== 'SUSPENDED') {
    return null;
  }
  const reason = tenant.suspendedReason ? reasons[tenant.suspendedReason] : undefined;
  return (
    <div className="mb-3.5">
      {tenant.suspendedReason === 'payment_pending' ? (
        <State tone="wait" icon={Clock} wrap>
          {t('tenantState.paymentPending')}
        </State>
      ) : (
        <State tone="err" icon={Ban} wrap>
          {reason ? t(reason) : t('tenantState.suspended', { reason: tenant.suspendedReason ?? '' })}
        </State>
      )}
    </div>
  );
}
