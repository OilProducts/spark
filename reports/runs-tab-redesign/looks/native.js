// Run selector: a compact status list. One line per run, CR id as a mono tag, the unique end of the run id.
const splitTitle = (t) => { const m = t.match(/^(CR-\d{4}-\d{4}) · (.*)$/); return m ? [m[1], m[2]] : [null, t] }
const STATUS_TEXT = { completed: 'Completed', failed: 'Failed', running: 'Running', waiting: 'Needs input', canceled: 'Canceled' }
function runsListHtml() {
  return `<div class="rs-head"><span class="rs-title">Runs</span><span class="rs-count">213</span><button class="btn rs-new">New run</button></div>
  <div class="rl-search"><input placeholder="Search runs  ⌘K"/></div>
  <div class="rs-tabs"><a class="on">All</a><a>Active <b>2</b></a><a>Failed</a></div>` +
  RUNS.map((r) => { const [cr, t] = splitTitle(r.title); return `
    <div data-id="${r.id}" class="rs-row ${r.current ? 'sel' : ''}">
      <div class="rs-l1">${cr ? `<span class="rs-cr">${cr}</span>` : ''}<span class="rs-t">${esc(t)}</span></div>
      <div class="rs-l2"><span class="rs-st ${r.status}">${STATUS_TEXT[r.status]}</span><span>${r.flow}</span><span class="rs-sp"></span><span>${r.when.replace(/ \d+:\d+$/, '')}</span><span>${r.dur}</span></div>
    </div>` }).join('')
}
