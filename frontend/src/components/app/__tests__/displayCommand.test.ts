import { describe, expect, it } from 'vitest'

import { stripShellWrapper } from '@/components/app/transcript/displayCommand'

describe('stripShellWrapper', () => {
    it.each([
        ["/bin/zsh -lc 'git status --short'", 'git status --short'],
        ['/bin/bash -lc "npm --prefix frontend run build"', 'npm --prefix frontend run build'],
        ['bash -c "echo hi"', 'echo hi'],
        ['sh -c ls', 'ls'],
        ['/usr/bin/zsh -l -c \'cat AGENTS.md\'', 'cat AGENTS.md'],
        ['bash --login -c "just test"', 'just test'],
        ["  /bin/zsh -lc 'ls'  ", 'ls'],
    ])('strips the wrapper from %s', (input, expected) => {
        expect(stripShellWrapper(input)).toBe(expected)
    })

    it('unquotes the way the shell would', () => {
        // '\'' is how a single quote is written inside single quotes.
        expect(stripShellWrapper("/bin/zsh -lc 'echo '\\''quoted'\\'' && rg -n \"a|b\" src'"))
            .toBe('echo \'quoted\' && rg -n "a|b" src')
        // Inside double quotes only \" \\ \$ and \` are escapes; other backslashes stay.
        expect(stripShellWrapper('bash -lc "node -e \\"require(1)\\" \\$HOME \\n"')).toBe('node -e "require(1)" $HOME \\n')
        // Newlines inside the quotes survive.
        expect(stripShellWrapper("/bin/zsh -lc 'set -e\nls\n'")).toBe('set -e\nls')
    })

    it.each([
        'git status --short',
        "cd repo && /bin/zsh -lc 'ls'",
        "/bin/zsh -lc 'ls' extra",
        "/bin/zsh -l 'ls'",
        "/bin/zsh -o pipefail -c 'ls'",
        "/bin/zsh -lc 'unterminated",
        "/bin/zsh -lc ''",
        "/bin/zsh -lc $'ansi\\tquoted'",
        '/bin/python -c "print(1)"',
    ])('leaves %s untouched', (input) => {
        expect(stripShellWrapper(input)).toBe(input)
    })
})
