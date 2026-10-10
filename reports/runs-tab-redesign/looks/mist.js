// Run selector: a macOS source list. Sections by state, a status symbol, one-line titles, accent pill selection.
const SYM = { completed: '✓', failed: '!', running: '◉', waiting: '?', canceled: '–' }
function runsListHtml() {
  const groups = [['Needs Input', RUNS.filter((r) => r.status === 'waiting')], ['Running', RUNS.filter((r) => r.status === 'running')],
    ['Recent', RUNS.filter((r) => !['waiting', 'running'].includes(r.status))]]
  return `<div class="mi-top"><input placeholder="Search"/></div>` + groups.filter(([, rs]) => rs.length).map(([g, rs]) =>
    `<div class="mi-sec">${g}</div>` + rs.map((r) => `
    <div data-id="${r.id}" class="mi-row ${r.current ? 'sel' : ''} ${r.status}">
      <span class="mi-sym">${SYM[r.status]}</span><span class="mi-t">${esc(r.title)}</span><span class="mi-d">${r.dur && r.dur !== '—' ? r.dur : ''}</span>
      <span class="mi-sub">${r.flow} · ${r.when}</span>
    </div>`).join('')).join('')
}
