// Run selector: borderless, title-first in a light weight. A hairline tick per visit hints at the run's shape.
const STATUS_WORD = { completed: '', failed: 'Failed', running: 'Running', waiting: 'Needs input', canceled: 'Canceled' }
function ticks(code) {
  return code ? `<span class="ai-ticks">${[...code].slice(-48).map((c) => `<i class="${{ f: 'w', e: 'e', '?': 'w', '.': 'r' }[c] || ''}"></i>`).join('')}</span>` : ''
}
function runsListHtml() {
  return `<div class="ai-top"><input placeholder="Search"/></div>` + RUNS.map((r) => `
    <div data-id="${r.id}" class="ai-row ${r.current ? 'sel' : ''} ${r.status}">
      <div class="ai-t">${esc(r.title)}</div>
      <div class="ai-m">${r.flow} · ${r.when}${r.dur && r.dur !== '—' ? ' · ' + r.dur : ''}${STATUS_WORD[r.status] ? ` · <span class="ai-s">${STATUS_WORD[r.status]}</span>` : ''}</div>
      ${ticks(r.strip)}
    </div>`).join('')
}
