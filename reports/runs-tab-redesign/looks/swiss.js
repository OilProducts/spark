// Run selector: an index grouped by day. Title carries the weight; status is a word, and only exceptions get colour.
const splitTitle = (t) => { const m = t.match(/^(CR-\d{4}-\d{4}) · (.*)$/); return m ? [m[1], m[2]] : ['', t] }
const DAY = (w) => /now|ago/.test(w) ? 'Today' : /Yesterday/.test(w) ? 'Yesterday' : 'Earlier'
const WORD = { completed: '', failed: 'Failed', running: 'Running', waiting: 'Needs input', canceled: 'Canceled' }
function runsListHtml() {
  const days = ['Today', 'Yesterday', 'Earlier']
  return `<div class="sw-top"><div class="sw-h">Runs</div><input placeholder="Search"/></div>` + days.map((d) => {
    const rs = RUNS.filter((r) => DAY(r.when) === d); if (!rs.length) return ''
    return `<div class="sw-day">${d}</div>` + rs.map((r) => { const [cr, t] = splitTitle(r.title); return `
      <div data-id="${r.id}" class="sw-row ${r.current ? 'sel' : ''} ${r.status}">
        <div class="sw-t">${esc(t)}</div>
        <div class="sw-m">${cr ? cr + ' · ' : ''}${r.flow} · ${r.dur}${WORD[r.status] ? ` · <b>${WORD[r.status]}</b>` : ''}</div>
      </div>` }).join('')
  }).join('')
}
