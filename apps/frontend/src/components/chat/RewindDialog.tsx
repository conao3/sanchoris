import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components"
import { t } from "../../i18n/index.ts"
import { Button } from "../Button.tsx"
import { ActionError } from "../ui.tsx"

export function RewindDialog({
  isOpen,
  onOpenChange,
  onRewind,
  busy,
  error,
}: {
  isOpen: boolean
  onOpenChange: (open: boolean) => void
  onRewind: (restoreFiles: boolean) => void
  busy: boolean
  error?: { message: string } | null
}) {
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className="fixed inset-0 z-30 flex items-start justify-center bg-ink/20 px-4 pt-[18vh]"
    >
      <Modal className="w-full max-w-[480px] rounded-[8px] border border-line2 bg-bg">
        <Dialog role="alertdialog" className="p-5 outline-none">
          <Heading slot="title" className="m-0 mb-2 text-base font-semibold">
            {t("rewind.title")}
          </Heading>
          <p className="text-sm text-body">{t("rewind.body")}</p>
          <ActionError error={error} className="mt-3" />
          <div className="mt-5 flex flex-wrap justify-end gap-2">
            <Button kbd="Esc" onPress={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              reason={busy ? t("rewind.busy") : undefined}
              onPress={() => onRewind(true)}
            >
              {t("rewind.restoreFiles")}
            </Button>
            <Button
              variant="primary"
              autoFocus
              reason={busy ? t("rewind.busy") : undefined}
              onPress={() => onRewind(false)}
            >
              {t("rewind.keepFiles")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  )
}
