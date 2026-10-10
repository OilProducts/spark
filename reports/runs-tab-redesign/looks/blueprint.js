// Run selector: flow-shaped. Each run shows the flow it ran as a strip of node-kind shapes, coloured by how each went.
const splitTitle = (t) => { const m = t.match(/^(CR-\d{4}-\d{4}) · (.*)$/); return m ? [m[1], m[2]] : ['', t] }
const STRIPS = {  // illustrative per-run outcome strips; the selected run's is real
  'run-18d999991f64': 'ooosa.', 'run-18d9f2a10b77': 'oa?', 'run-18d9543bcf14': 'ooosfsatttx', 'run-18d93c056079': 'ooosfsaeosfsaatttx',
  'run-18d92e0b0bfd': 'ooosfsatttx', 'run-18d86d7c2aa0': 'ooos-', 'run-18d81a398250': 'ooosaeeee', 'run-18c4a0b2d3e1': 'osaaae',
}
function strip(code) {  // o ok · s subflow ok · a agent ok · f sent back · e failed · t tool · x exit · . running · ? waiting · - canceled
  return [...code].map((c) => `<i class="sh ${{ o: 'ok sq', s: 'ok di', a: 'ok ci', f: 'warn ci', e: 'err sq', t: 'ok sq', x: 'ok ci', '.': 'run ci', '?': 'wait di', '-': 'off sq' }[c]}"></i>`).join('')
}
const STATUS_TEXT = { completed: 'COMPLETED', failed: 'FAILED', running: 'RUNNING', waiting: 'NEEDS INPUT', canceled: 'CANCELED' }
function runsListHtml() {
  return `<div class="bp-head">RUNS <span>213</span></div><div class="rl-search"><input placeholder="Search"/></div>` +
  RUNS.map((r) => { const [cr, t] = splitTitle(r.title); return `
    <div data-id="${r.id}" class="bp-row ${r.current ? 'sel' : ''}">
      <div class="bp-l1"><span class="bp-cr">${cr || r.flow.toUpperCase()} · ${r.when} · ${r.dur}</span><span class="bp-st ${r.status}">${STATUS_TEXT[r.status]}</span></div>
      <div class="bp-t">${esc(t)}</div>
      <div class="bp-strip">${strip(r.strip ?? STRIPS[r.id] ?? '')}</div>
    </div>` }).join('')
}
