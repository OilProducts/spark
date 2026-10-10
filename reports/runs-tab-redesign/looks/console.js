// Run selector: a dense table. Status glyph, CR id, title, age, duration, and the unique tail of the run id.
const splitTitle = (t) => { const m = t.match(/^(CR-\d{4}-\d{4}) · (.*)$/); return m ? [m[1], m[2]] : ['', t] }
const GLYPH = { completed: '●', failed: '✕', running: '◐', waiting: '◆', canceled: '○' }
function runsListHtml() {
  return `<div class="cs-bar"><input placeholder="/ filter   status:failed   flow:implement"/></div>
  <div class="cs-cols"><span></span><span>run</span><span>age</span><span>dur</span></div>` +
  RUNS.map((r) => { const [cr, t] = splitTitle(r.title); return `
    <div data-id="${r.id}" class="cs-row ${r.current ? 'sel' : ''} ${r.status}">
      <span class="cs-g">${GLYPH[r.status]}</span>
      <span class="cs-name"><span class="cs-t">${cr ? `<span class="cs-cr">${cr}</span> ` : ''}${esc(t)}</span><span class="cs-id">${r.flow.toLowerCase().replace(/[^a-z0-9]+/g, '-')} · …${r.id.slice(-6)}</span></span>
      <span class="cs-n">${r.when.replace(/^Yesterday.*/, '1d').replace(' ago', '')}</span><span class="cs-n">${r.dur}</span>
    </div>` }).join('') + `<div class="cs-foot"><kbd>j</kbd><kbd>k</kbd> move · <kbd>↵</kbd> open · <kbd>/</kbd> filter</div>`
}
