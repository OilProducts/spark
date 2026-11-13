import { useState, type FormEvent } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { InlineError } from '@/components/app/inline-error'
import { Well } from '@/components/app/well'
import { loadEnvironments, normalizeOrigin, openEnvironment, saveEnvironments, type SavedEnvironment } from '@/lib/environments'

// `navigate` replaces the browser navigation in tests; jsdom's location cannot be spied on.
export function EnvironmentsCard({ navigate }: { navigate?: (url: string) => void }) {
    const [entries, setEntries] = useState(loadEnvironments)
    const [formError, setFormError] = useState<string | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [status, setStatus] = useState('')
    const [opening, setOpening] = useState(false)

    const persist = (next: SavedEnvironment[]) => {
        try {
            saveEnvironments(next)
            setEntries(next)
            return true
        } catch {
            setError('Could not save environments in this browser.')
            return false
        }
    }

    const add = (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault()
        const form = event.currentTarget
        const data = new FormData(form)
        const origin = normalizeOrigin(String(data.get('origin')))
        const expected_id = String(data.get('expected_id')).trim()
        if (!origin) return setFormError('Enter an http:// or https:// origin with no path, query or credentials, such as http://127.0.0.1:8000.')
        if (!expected_id) return setFormError('Enter the expected environment ID.')
        // An origin is saved once; adding it again replaces the earlier entry.
        if (persist([...entries.filter((entry) => entry.origin !== origin), { origin, expected_id, label: String(data.get('label')).trim() }])) {
            form.reset()
            setFormError(null)
        }
    }

    const open = async (entry: SavedEnvironment) => {
        const name = entry.label || entry.origin
        setOpening(true)
        setError(null)
        setStatus(`Checking ${name}…`)
        try {
            await openEnvironment(entry, navigate)
            setStatus(isTauri() ? `Opened ${name} in your browser.` : `Opening ${name}…`)
        } catch (error) {
            setStatus('')
            setError(error instanceof Error ? error.message : `Could not open ${name}.`)
        } finally {
            setOpening(false)
        }
    }

    return <Card className="gap-4 py-4">
        <CardHeader className="px-4"><h2 className="text-base font-semibold">Environments</h2></CardHeader>
        <CardContent className="space-y-3 px-4 text-sm">
            <p className="text-muted-foreground">Other Spark environments to open from here, saved in this browser only. Open first checks that the origin still serves the expected environment ID, shown at the origin’s /environment.</p>
            {entries.length > 0 && <ul className="space-y-2">
                {entries.map((entry) => {
                    const name = entry.label || entry.origin
                    return <li key={entry.origin}>
                        <Well className="flex flex-wrap items-center justify-between gap-2 px-3 py-2">
                            <div className="min-w-0 space-y-1">
                                <div className="font-medium text-foreground">{name}</div>
                                <div className="break-all text-xs text-muted-foreground">{entry.origin} · {entry.expected_id}</div>
                            </div>
                            <div className="flex flex-wrap gap-2">
                                <Button size="sm" variant="outline" aria-label={`Open ${name}`} disabled={opening} onClick={() => void open(entry)}>Open</Button>
                                <Button size="sm" variant="ghost" aria-label={`Remove ${name}`} onClick={() => persist(entries.filter((other) => other !== entry))}>Remove</Button>
                            </div>
                        </Well>
                    </li>
                })}
            </ul>}
            {status && <p role="status">{status}</p>}
            {error && <InlineError>{error}</InlineError>}
            <form className="space-y-3" aria-label="Add environment" onSubmit={add}>
                <Field><FieldLabel htmlFor="environment-origin">Origin</FieldLabel>
                    <Input id="environment-origin" name="origin" placeholder="http://127.0.0.1:8000" autoComplete="off" aria-describedby={formError ? 'environment-form-error' : undefined} />
                </Field>
                <Field><FieldLabel htmlFor="environment-expected-id">Expected environment ID</FieldLabel>
                    <Input id="environment-expected-id" name="expected_id" autoComplete="off" aria-describedby={formError ? 'environment-form-error' : undefined} />
                </Field>
                <Field><FieldLabel htmlFor="environment-label">Label (optional)</FieldLabel>
                    <Input id="environment-label" name="label" autoComplete="off" />
                </Field>
                {formError && <p id="environment-form-error" role="alert" className="text-destructive">{formError}</p>}
                <Button type="submit" size="sm">Add environment</Button>
            </form>
        </CardContent>
    </Card>
}
