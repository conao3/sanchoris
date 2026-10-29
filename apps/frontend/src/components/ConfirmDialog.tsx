import type { LucideIcon } from 'lucide-react';
import type { FormEvent, ReactNode } from 'react';
import { Dialog, Heading, Modal, ModalOverlay } from 'react-aria-components';
import { t } from '../i18n';
import { Button } from './Button';
import { ActionError } from './ui';

type ConfirmDialogProps = {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children: ReactNode;
  confirmLabel: string;
  confirmIcon?: LucideIcon;
  onConfirm: () => void;
  isConfirmDisabled?: boolean;
  focusConfirm?: boolean;
  error?: { message: string } | null;
};

export function ConfirmDialog({
  isOpen,
  onOpenChange,
  title,
  children,
  confirmLabel,
  confirmIcon,
  onConfirm,
  isConfirmDisabled,
  focusConfirm = true,
  error,
}: ConfirmDialogProps) {
  function submit(event: FormEvent) {
    event.preventDefault();
    if (!isConfirmDisabled) {
      onConfirm();
    }
  }

  return (
    <ModalOverlay isOpen={isOpen} onOpenChange={onOpenChange} isDismissable className="fixed inset-0 z-30 flex items-start justify-center bg-ink/20 px-4 pt-[18vh]">
      <Modal className="w-full max-w-[440px] rounded-[8px] border border-line2 bg-bg">
        <Dialog role="alertdialog" className="outline-none">
          <form onSubmit={submit} className="p-5">
            <Heading slot="title" className="m-0 mb-2 text-base font-semibold">
              {title}
            </Heading>
            <div className="text-sm text-body">{children}</div>
            <ActionError error={error} className="mt-3" />
            <div className="mt-5 flex justify-end gap-2">
              <Button kbd="Esc" onPress={() => onOpenChange(false)}>
                {t('common.cancel')}
              </Button>
              <Button type="submit" variant="danger" icon={confirmIcon} kbd="↵" isDisabled={isConfirmDisabled} autoFocus={focusConfirm}>
                {confirmLabel}
              </Button>
            </div>
          </form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
