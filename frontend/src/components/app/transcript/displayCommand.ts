// Agents run commands through a login shell, so a tool call's command often reads
// `/bin/zsh -lc '<cmd>'`. For display we show only `<cmd>`, unquoted the way the shell
// would; anything we cannot parse with confidence is left exactly as it came.

const SHELL_PROGRAM = /^(?:\/(?:usr\/(?:local\/)?)?bin\/)?(?:sh|bash|zsh|dash|ksh)$/

/** Splits one line of shell into words, applying quoting; null when quotes are unbalanced. */
function splitShellWords(input: string): string[] | null {
    const words: string[] = []
    let word: string | null = null
    let quote: '"' | "'" | null = null
    for (let index = 0; index < input.length; index += 1) {
        const char = input[index]
        if (quote === "'") {
            if (char === "'") quote = null
            else word += char
            continue
        }
        if (quote === '"') {
            if (char === '"') {
                quote = null
            } else if (char === '\\' && index + 1 < input.length) {
                const next = input[index + 1]
                if (next === '\n') {
                    index += 1
                } else if ('"\\$`'.includes(next)) {
                    word += next
                    index += 1
                } else {
                    word += char
                }
            } else {
                word += char
            }
            continue
        }
        if (char === "'" || char === '"') {
            // ANSI-C quoting ($'...') has escape rules we do not implement.
            if (char === "'" && word !== null && word.endsWith('$')) return null
            quote = char
            word ??= ''
            continue
        }
        if (char === '\\') {
            if (index + 1 >= input.length) return null
            const next = input[index + 1]
            index += 1
            if (next !== '\n') word = (word ?? '') + next
            continue
        }
        if (/\s/.test(char)) {
            if (word !== null) {
                words.push(word)
                word = null
            }
            continue
        }
        word = (word ?? '') + char
    }
    if (quote !== null) return null
    if (word !== null) words.push(word)
    return words
}

/**
 * `/bin/zsh -lc '<cmd>'`, `bash -lc "<cmd>"`, `sh -c <cmd>` and similar become `<cmd>`.
 * Anything else, including a wrapper with trailing arguments, is returned untouched.
 */
export function stripShellWrapper(command: string): string {
    const words = splitShellWords(command.trim())
    if (!words || words.length < 3 || !SHELL_PROGRAM.test(words[0])) return command
    // Only bare flags may sit between the shell and the script (`-lc`, `-l -c`, `--login -c`);
    // a flag that takes a value, or extra positional arguments, means we leave it alone.
    const flags = words.slice(1, -1)
    const isFlag = (flag: string) => /^-{1,2}[a-zA-Z][a-zA-Z-]*$/.test(flag)
    const requestsScript = (flag: string) => /^-[a-zA-Z]*c[a-zA-Z]*$/.test(flag)
    if (!flags.every(isFlag) || !flags.some(requestsScript)) return command
    const inner = words[words.length - 1].trim()
    return inner.length > 0 ? inner : command
}
