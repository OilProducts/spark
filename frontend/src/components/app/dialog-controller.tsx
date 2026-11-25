import {
  type ComponentProps,
  createContext,
  useContext,
  useLayoutEffect,
  useMemo,
  useState,
  useSyncExternalStore,
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
import { Field, FieldDescription, FieldError, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { getActiveEnvironmentId, subscribeActiveEnvironment } from '@/lib/activeEnvironment'

type ButtonVariant = ComponentProps<typeof Button>['variant']

type ConfirmDialogOptions = {
  title: string
  description?: string | null
  confirmLabel?: string
  cancelLabel?: string
  confirmVariant?: ButtonVariant
  /** Asks whether to leave the current view, such as for a switch to another environment awaiting approval. */
  leaving?: boolean
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
  /** A live note under the input; `ok: false` blocks confirming. */
  check?: (value: string) => { message: string; ok: boolean } | null
}

type DialogController = {
  alert: (options: AlertDialogOptions) => Promise<void>
  confirm: (options: ConfirmDialogOptions) => Promise<boolean>
  prompt: (options: PromptDialogOptions) => Promise<string | null>
  /** Brings an open question about leaving over a layer opened after it, such as an environment's error gate. */
  raiseLeaving: () => void
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
  value: string
  resolve: (value: string | null) => void
}

// Each dialog is its own layer, so one asked while another layer is open, such as an environment's error gate over a
// question the workspace left open, opens over it. It answers for the environment shown as it was asked.
type DialogRequest = (AlertDialogRequest | ConfirmDialogRequest | PromptDialogRequest) & { id: number; environmentId: string | null }
let dialogRequests = 0

/** Answers a dialog closed without confirming it. */
function resolveClosed(dialog: DialogRequest) {
  if (dialog.kind === 'alert') {
    dialog.resolve()
  } else if (dialog.kind === 'confirm') {
    dialog.resolve(false)
  } else {
    dialog.resolve(null)
  }
}

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
  raiseLeaving: () => {},
}

export function DialogProvider({ children }: PropsWithChildren) {
  // Open dialogs, oldest first: one asked while another is open, such as a switch's confirmation over a prompt, opens
  // over it and leaves it, with what was entered, as it was.
  const [dialogs, setDialogs] = useState<DialogRequest[]>([])
  const activeEnvironmentId = useSyncExternalStore(subscribeActiveEnvironment, getActiveEnvironmentId)
  // A switch to another environment closes, unanswered, the dialogs asked in the one before, such as a deletion's
  // confirmation, before they show over the workspace they would now act in.
  useLayoutEffect(() => {
    setDialogs((open) => {
      const stale = open.filter((dialog) => dialog.environmentId !== activeEnvironmentId)
      stale.forEach(resolveClosed)
      return stale.length ? open.filter((dialog) => dialog.environmentId === activeEnvironmentId) : open
    })
  }, [activeEnvironmentId])

  const controller = useMemo<DialogController>(() => {
    const push = (dialog: DialogRequest) => setDialogs((open) => [...open, dialog])
    return {
      alert: (options) => new Promise<void>((resolve) => {
        push({
          kind: 'alert',
          id: ++dialogRequests,
          environmentId: getActiveEnvironmentId(),
          confirmLabel: 'OK',
          confirmVariant: 'default',
          ...options,
          resolve,
        })
      }),
      confirm: (options) => new Promise<boolean>((resolve) => {
        push({
          kind: 'confirm',
          id: ++dialogRequests,
          environmentId: getActiveEnvironmentId(),
          confirmLabel: 'Confirm',
          cancelLabel: 'Cancel',
          confirmVariant: 'default',
          ...options,
          resolve,
        })
      }),
      prompt: (options) => new Promise<string | null>((resolve) => {
        push({
          kind: 'prompt',
          id: ++dialogRequests,
          environmentId: getActiveEnvironmentId(),
          confirmLabel: 'Save',
          cancelLabel: 'Cancel',
          confirmVariant: 'default',
          multiline: false,
          requireInput: false,
          ...options,
          value: options.defaultValue ?? '',
          resolve,
        })
      }),
      raiseLeaving: () => setDialogs((open) => {
        const leaving = (dialog: DialogRequest) => dialog.kind === 'confirm' && dialog.leaving === true
        return open.some(leaving)
          ? [...open.filter((dialog) => !leaving(dialog)), ...open.filter(leaving).map((dialog) => ({ ...dialog, id: ++dialogRequests }))]
          : open
      }),
    }
  }, [])

  // Answers one dialog and closes it; closing an answered one does nothing.
  const settle = (id: number, answer: (dialog: DialogRequest) => void) => {
    setDialogs((open) => {
      const dialog = open.find((entry) => entry.id === id)
      if (!dialog) return open
      answer(dialog)
      return open.filter((entry) => entry.id !== id)
    })
  }
  const closeDialog = (id: number) => settle(id, resolveClosed)
  const confirmDialog = (id: number) => settle(id, (dialog) => {
    if (dialog.kind === 'alert') {
      dialog.resolve()
    } else if (dialog.kind === 'confirm') {
      dialog.resolve(true)
    } else {
      dialog.resolve(dialog.requireInput ? dialog.value.trim() : dialog.value)
    }
  })
  const setPromptValue = (id: number, value: string) => {
    setDialogs((open) => open.map((dialog) => (dialog.id === id && dialog.kind === 'prompt' ? { ...dialog, value } : dialog)))
  }

  return (
    <DialogControllerContext.Provider value={controller}>
      {children}
      {dialogs.map((dialog) => {
        if (dialog.kind !== 'prompt') {
          return (
            <AlertDialog
              key={dialog.id}
              open
              onOpenChange={(isOpen) => {
                if (!isOpen) {
                  closeDialog(dialog.id)
                }
              }}
            >
              <AlertDialogContent
                data-testid="shared-dialog"
                className="sm:max-w-md"
              >
                <AlertDialogHeader>
                  <AlertDialogTitle data-testid="shared-dialog-title">
                    {dialog.title}
                  </AlertDialogTitle>
                  {dialog.description ? (
                    <AlertDialogDescription data-testid="shared-dialog-description">
                      {dialog.description}
                    </AlertDialogDescription>
                  ) : null}
                </AlertDialogHeader>
                <AlertDialogFooter>
                  {dialog.kind === 'confirm' ? (
                    <AlertDialogCancel data-testid="shared-dialog-cancel">
                      {dialog.cancelLabel || 'Cancel'}
                    </AlertDialogCancel>
                  ) : null}
                  <AlertDialogAction
                    data-testid="shared-dialog-confirm"
                    onClick={() => confirmDialog(dialog.id)}
                    asChild
                  >
                    <Button variant={dialog.confirmVariant}>
                      {dialog.confirmLabel || 'OK'}
                    </Button>
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          )
        }
        const promptValue = dialog.value
        const promptCheck = promptValue.trim() ? dialog.check?.(promptValue) ?? null : null
        const promptConfirmDisabled = (dialog.requireInput === true && promptValue.trim().length === 0) || promptCheck?.ok === false
        const inputId = `shared-dialog-input-${dialog.id}`
        const checkId = `shared-dialog-check-${dialog.id}`
        return (
          <Dialog
            key={dialog.id}
            open
            onOpenChange={(isOpen) => {
              if (!isOpen) {
                closeDialog(dialog.id)
              }
            }}
          >
            <DialogContent
              data-testid="shared-dialog"
              className="sm:max-w-md"
            >
              <DialogHeader>
                <DialogTitle data-testid="shared-dialog-title">{dialog.title}</DialogTitle>
                {dialog.description ? (
                  <DialogDescription data-testid="shared-dialog-description">
                    {dialog.description}
                  </DialogDescription>
                ) : null}
              </DialogHeader>
              <form
                className="space-y-4"
                onSubmit={(event) => {
                  event.preventDefault()
                  if (promptConfirmDisabled) {
                    return
                  }
                  confirmDialog(dialog.id)
                }}
              >
                <Field>
                  <FieldLabel htmlFor={inputId}>{dialog.label}</FieldLabel>
                  {dialog.multiline ? (
                    <Textarea
                      id={inputId}
                      data-testid="shared-dialog-input"
                      value={promptValue}
                      onChange={(event) => setPromptValue(dialog.id, event.target.value)}
                      placeholder={dialog.placeholder}
                      rows={4}
                    />
                  ) : (
                    <Input
                      id={inputId}
                      data-testid="shared-dialog-input"
                      value={promptValue}
                      onChange={(event) => setPromptValue(dialog.id, event.target.value)}
                      placeholder={dialog.placeholder}
                      autoFocus
                      aria-invalid={promptCheck?.ok === false || undefined}
                      aria-describedby={promptCheck ? checkId : undefined}
                    />
                  )}
                  {promptCheck ? (
                    promptCheck.ok
                      ? <FieldDescription id={checkId} data-testid="shared-dialog-check">{promptCheck.message}</FieldDescription>
                      : <FieldError id={checkId} data-testid="shared-dialog-check">{promptCheck.message}</FieldError>
                  ) : null}
                </Field>
                <DialogFooter>
                  <Button
                    type="button"
                    data-testid="shared-dialog-cancel"
                    variant="outline"
                    onClick={() => closeDialog(dialog.id)}
                  >
                    {dialog.cancelLabel || 'Cancel'}
                  </Button>
                  <Button
                    type="submit"
                    data-testid="shared-dialog-confirm"
                    variant={dialog.confirmVariant}
                    disabled={promptConfirmDisabled}
                  >
                    {dialog.confirmLabel || 'Save'}
                  </Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>
        )
      })}
    </DialogControllerContext.Provider>
  )
}

export function useDialogController() {
  return useContext(DialogControllerContext) ?? fallbackDialogController
}
