import {
  type ComponentProps,
  createContext,
  useContext,
  useMemo,
  useState,
  type PropsWithChildren,
} from 'react'

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'

type ButtonVariant = ComponentProps<typeof Button>['variant']

type ConfirmDialogOptions = {
  title: string
  description?: string | null
  confirmLabel?: string
  cancelLabel?: string
  confirmVariant?: ButtonVariant
}

type AlertDialogOptions = {
  title: string
  description?: string | null
  confirmLabel?: string
  confirmVariant?: ButtonVariant
}

type PromptDialogOptions = {
  title: string
  description?: string | null
  label: string
  defaultValue?: string
  placeholder?: string
  confirmLabel?: string
  cancelLabel?: string
  confirmVariant?: ButtonVariant
  multiline?: boolean
  requireInput?: boolean
}

type DialogController = {
  alert: (options: AlertDialogOptions) => Promise<void>
  confirm: (options: ConfirmDialogOptions) => Promise<boolean>
  prompt: (options: PromptDialogOptions) => Promise<string | null>
}

type AlertDialogRequest = AlertDialogOptions & {
  kind: 'alert'
  resolve: () => void
}

type ConfirmDialogRequest = ConfirmDialogOptions & {
  kind: 'confirm'
  resolve: (value: boolean) => void
}

type PromptDialogRequest = PromptDialogOptions & {
  kind: 'prompt'
  resolve: (value: string | null) => void
}

type DialogRequest = AlertDialogRequest | ConfirmDialogRequest | PromptDialogRequest

const DialogControllerContext = createContext<DialogController | null>(null)

const formatFallbackMessage = (title: string, description?: string | null) => (
  description || title
)

const fallbackDialogController: DialogController = {
  alert: async ({ title, description }) => {
    if (typeof window !== 'undefined') {
      window.alert(formatFallbackMessage(title, description))
    }
  },
  confirm: async ({ title, description }) => {
    if (typeof window === 'undefined') {
      return false
    }
    return window.confirm(formatFallbackMessage(title, description))
  },
  prompt: async ({ title, description, defaultValue = '', requireInput = false }) => {
    if (typeof window === 'undefined') {
      return null
    }
    const nextValue = window.prompt(formatFallbackMessage(title, description), defaultValue)
    if (nextValue === null) {
      return null
    }
    const trimmedValue = nextValue.trim()
    if (requireInput && trimmedValue.length === 0) {
      return null
    }
    return trimmedValue
  },
}

export function DialogProvider({ children }: PropsWithChildren) {
  const [currentDialog, setCurrentDialog] = useState<DialogRequest | null>(null)
  const [promptValue, setPromptValue] = useState('')

  const controller = useMemo<DialogController>(() => ({
    alert: (options) => new Promise<void>((resolve) => {
      setCurrentDialog({
        kind: 'alert',
        confirmLabel: 'OK',
        confirmVariant: 'default',
        ...options,
        resolve,
      })
    }),
    confirm: (options) => new Promise<boolean>((resolve) => {
      setCurrentDialog({
        kind: 'confirm',
        confirmLabel: 'Confirm',
        cancelLabel: 'Cancel',
        confirmVariant: 'default',
        ...options,
        resolve,
      })
    }),
    prompt: (options) => new Promise<string | null>((resolve) => {
      setPromptValue(options.defaultValue ?? '')
      setCurrentDialog({
        kind: 'prompt',
        confirmLabel: 'Save',
        cancelLabel: 'Cancel',
        confirmVariant: 'default',
        multiline: false,
        requireInput: false,
        ...options,
        resolve,
      })
    }),
  }), [])

  const closeDialog = () => {
    setCurrentDialog((dialog) => {
      if (!dialog) {
        return dialog
      }
      if (dialog.kind === 'alert') {
        dialog.resolve()
      } else if (dialog.kind === 'confirm') {
        dialog.resolve(false)
      } else {
        dialog.resolve(null)
      }
      return null
    })
    setPromptValue('')
  }

  const confirmDialog = () => {
    setCurrentDialog((dialog) => {
      if (!dialog) {
        return dialog
      }
      if (dialog.kind === 'alert') {
        dialog.resolve()
      } else if (dialog.kind === 'confirm') {
        dialog.resolve(true)
      } else {
        dialog.resolve(dialog.requireInput ? promptValue.trim() : promptValue)
      }
      return null
    })
    setPromptValue('')
  }

  const promptRequiresInput = currentDialog?.kind === 'prompt' && currentDialog.requireInput === true
  const promptConfirmDisabled = promptRequiresInput && promptValue.trim().length === 0

  return (
    <DialogControllerContext.Provider value={controller}>
      {children}
      {currentDialog && currentDialog.kind !== 'prompt' ? (
        <AlertDialog
          open
          onOpenChange={(isOpen) => {
            if (!isOpen) {
              closeDialog()
            }
          }}
        >
          <AlertDialogContent
            data-testid="shared-dialog"
            className="sm:max-w-md"
          >
            <AlertDialogHeader>
              <AlertDialogTitle data-testid="shared-dialog-title">
                {currentDialog.title}
              </AlertDialogTitle>
              {currentDialog.description ? (
                <AlertDialogDescription data-testid="shared-dialog-description">
                  {currentDialog.description}
                </AlertDialogDescription>
              ) : null}
            </AlertDialogHeader>
            <AlertDialogFooter>
              {currentDialog.kind === 'confirm' ? (
                <AlertDialogCancel data-testid="shared-dialog-cancel">
                  {currentDialog.cancelLabel || 'Cancel'}
                </AlertDialogCancel>
              ) : null}
              <AlertDialogAction
                data-testid="shared-dialog-confirm"
                onClick={confirmDialog}
                asChild
              >
                <Button variant={currentDialog.confirmVariant}>
                  {currentDialog.confirmLabel || 'OK'}
                </Button>
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
      <Dialog
        open={currentDialog?.kind === 'prompt'}
        onOpenChange={(isOpen) => {
          if (!isOpen) {
            closeDialog()
          }
        }}
      >
        {currentDialog?.kind === 'prompt' ? (
          <DialogContent
            data-testid="shared-dialog"
            className="sm:max-w-md"
          >
            <DialogHeader>
              <DialogTitle data-testid="shared-dialog-title">{currentDialog.title}</DialogTitle>
              {currentDialog.description ? (
                <DialogDescription data-testid="shared-dialog-description">
                  {currentDialog.description}
                </DialogDescription>
              ) : null}
            </DialogHeader>
            {currentDialog.kind === 'prompt' ? (
              <form
                className="space-y-4"
                onSubmit={(event) => {
                  event.preventDefault()
                  if (promptConfirmDisabled) {
                    return
                  }
                  confirmDialog()
                }}
              >
                <Field>
                  <FieldLabel htmlFor="shared-dialog-input">{currentDialog.label}</FieldLabel>
                  {currentDialog.multiline ? (
                    <Textarea
                      id="shared-dialog-input"
                      data-testid="shared-dialog-input"
                      value={promptValue}
                      onChange={(event) => setPromptValue(event.target.value)}
                      placeholder={currentDialog.placeholder}
                      rows={4}
                    />
                  ) : (
                    <Input
                      id="shared-dialog-input"
                      data-testid="shared-dialog-input"
                      value={promptValue}
                      onChange={(event) => setPromptValue(event.target.value)}
                      placeholder={currentDialog.placeholder}
                      autoFocus
                    />
                  )}
                </Field>
                <DialogFooter>
                  <Button
                    type="button"
                    data-testid="shared-dialog-cancel"
                    variant="outline"
                    onClick={closeDialog}
                  >
                    {currentDialog.cancelLabel || 'Cancel'}
                  </Button>
                  <Button
                    type="submit"
                    data-testid="shared-dialog-confirm"
                    variant={currentDialog.confirmVariant}
                    disabled={promptConfirmDisabled}
                  >
                    {currentDialog.confirmLabel || 'Save'}
                  </Button>
                </DialogFooter>
              </form>
            ) : null}
          </DialogContent>
        ) : null}
      </Dialog>
    </DialogControllerContext.Provider>
  )
}

export function useDialogController() {
  return useContext(DialogControllerContext) ?? fallbackDialogController
}
