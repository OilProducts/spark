"""Extract real run records into runs.js for the generic runs-tab mockup.

Reads run directories under the desktop app's attractor/runs root (read-only) and
normalizes each into one shape: run facts, flow graph, and one entry per visit.
Reads the per-execution log layout (logs/<node>/executions/<idx>-<attempt>/) only;
older run formats are not supported.

    python3 extract.py            # writes runs.js next to this file
"""
import json, os, re
from datetime import datetime, timezone, timedelta

ROOT = os.path.expanduser('~/Library/Application Support/dev.spark.desktop/spark/attractor/runs')
RUNS = [  # (project dir, run id, note, replay-until-sequence)
    ('spark-9301d20d6c95', 'run-18d93c056079b9c0', 'Implement change · the run F was designed around', None),
    ('math-tuza-439ad834d67f', 'run-18c5496cf34c1940', 'Resolution program · math, failed: container would not start', None),
    ('understory-f64106c3bb2a', 'run-18c959a067423158', 'Implement spec · failed after 66 stages (backend)', None),
    ('spark-9301d20d6c95', 'run-18d94db5c0f0fce8', 'Merge change · completed, tool-heavy', None),
    ('spark-9301d20d6c95', 'c481dd7010a8503fc1360cd2abe85ceb', 'Implement Task · replayed while waiting on a question', 11),
    ('spark-9301d20d6c95', 'fadfb2efee5f739ef85dded968482371', 'Implement Task · child run opened on its own', None),
]
EDT = timezone(timedelta(hours=-4))
INFRA = re.compile(r'codergen backend failed|app-server|usage limit|Reconnecting|Container node worker|daemon', re.I)
SYSTEM_NS = ('_attractor', 'internal', 'graph', 'context.stack', 'execution_', 'current_node', 'outcome', 'last_', 'preferred_label')


def ts(s):
    return datetime.fromisoformat(s.replace('Z', '+00:00')[:26] + '+00:00' if '.' in s else s.replace('Z', '+00:00')).astimezone(EDT)


def clock(s):
    return ts(s).strftime('%H:%M:%S') if s else None


def read_json(path, default=None):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def read_lines(path):
    try:
        with open(path) as f:
            return [json.loads(l) for l in f if l.strip()]
    except OSError:
        return []


def read_text(path, limit=None):
    try:
        with open(path) as f:
            t = f.read()
        return t[:limit] if limit else t
    except OSError:
        return None


def short(s, n=400):
    s = re.sub(r'\s+', ' ', str(s or '')).strip()
    return s if len(s) <= n else s[:n - 1] + '…'


def strip_ctx(k):
    return k[len('context.'):] if k.startswith('context.') else k


def is_system(k):
    return k.startswith(SYSTEM_NS) or not k.startswith('context.')


def prompt_parts(text):
    """Split a rendered prompt into declared reads and the stage instructions."""
    if not text:
        return [], None
    reads, instr = [], text
    if 'Declared context reads:' in text:
        head, _, rest = text.partition('Declared context reads:')
        block, sep, after = rest.partition('Current stage task:')
        reads = [strip_ctx(l.split('=', 1)[0].strip()) for l in block.splitlines() if l.startswith('context.') and not l.startswith('context.stack.')]
        instr = after if sep else ''
    return reads, short(instr, 700) or None


def trim_rows(rows, head=7, tail=4):
    if len(rows) <= head + tail + 1:
        return rows
    return rows[:head] + [['more', f'{len(rows) - head - tail} more steps']] + rows[-tail:]


def rows_from_transcript(records):
    rows, prompt = [], None
    for r in records:
        if r.get('type') == 'turn_upsert' and r['turn'].get('id') == 'prompt':
            prompt = r['turn'].get('content')
        if r.get('type') != 'segment_upsert':
            continue
        seg = r['segment']
        k = seg.get('kind')
        if k == 'assistant_message' and seg.get('content'):
            rows.append(['msg', short(seg['content'], 360)])
        elif k == 'reasoning' and seg.get('content'):
            rows.append(['think', short(seg['content'].strip('*'), 80)])
        elif k == 'tool_call' and seg.get('tool_call'):
            tc = seg['tool_call']
            rows.append(['tool', short(tc.get('command') or tc.get('title'), 140)])
        elif k == 'request_user_input':
            q = (seg.get('request_user_input') or {}).get('questions') or [{}]
            rows.append(['ask', short(q[0].get('question'), 300)])
    return rows, prompt


def node_kind(cfg):
    k = (cfg or {}).get('kind', '')
    return {'agent_task': 'agent', 'tool': 'tool', 'subflow': 'subflow', 'start': 'start', 'exit': 'exit',
            'parallel': 'parallel', 'fan_in': 'join', 'human': 'human', 'wait.human': 'human'}.get(k, k or 'agent')


def extract(project, run_id, note, replay_until):
    d = os.path.join(ROOT, project, run_id)
    meta = read_json(os.path.join(d, 'run.json'), {})
    events = read_lines(os.path.join(d, 'events.jsonl'))
    if replay_until:
        events = [e for e in events if e['sequence'] < replay_until]
    flow = read_json(os.path.join(d, 'artifacts/flow/flow-definition.json'), {})
    nodes_raw = flow.get('nodes') or {}
    nodes = [{'id': k, 'label': (v.get('config') or {}).get('label') or v.get('label') or k.replace('_', ' ').title(),
              'kind': node_kind(v.get('config'))} for k, v in nodes_raw.items()]
    node_cfg = {k: v.get('config') or {} for k, v in nodes_raw.items()}
    edges = [{'from': e['from'], 'to': e['to'], 'label': e.get('label') or '', 'cond': e.get('condition') or ''} for e in flow.get('edges', [])]
    inputs = flow.get('inputs') or []
    launch = meta.get('launch_context') or {}

    # --- visits from the root journal ---
    stage_events = [e for e in events if e['type'] in ('StageStarted', 'StageCompleted', 'StageFailed')]
    visits, open_by_node = [], {}
    for e in stage_events:
        if e['type'] == 'StageStarted':
            v = {'i': len(visits), 'node': e['node_id'], 'stage': e.get('index'), 's': clock(e['emitted_at']), 'e': None,
                 'seq0': e['sequence'], 'seq1': None, 'outcome': 'running'}
            visits.append(v)
            open_by_node[e['node_id']] = v
        else:
            v = open_by_node.pop(e['node_id'], None)
            if not v:
                continue
            v['e'] = clock(e['emitted_at']); v['seq1'] = e['sequence']
            if e['type'] == 'StageCompleted':
                v['outcome'] = 'partial' if e.get('outcome') == 'partial_success' else 'success'
                v['summary'] = short(e.get('notes'), 420) or None
            else:
                v['outcome'] = 'failed'
                v['summary'] = short(e.get('error'), 420) if e.get('error') not in (None, '', 'stage_failed') else None
    # a failure the run moved past was routed (did not pass); the last one is the run's failure
    for k, v in enumerate(visits):
        if v['outcome'] == 'failed':
            v['outcome'] = 'fail' if k < len(visits) - 1 else 'error'

    # --- parallel branches: children of the fan-out visit ---
    for e in events:
        if e['type'] == 'ParallelBranchStarted':
            parent = next((v for v in reversed(visits) if v['node'] == e['node_id'] and v['seq0'] < e['sequence']), None)
            visits.append({'i': None, 'node': e['branch'], 'branchOf': parent['i'] if parent else None, 's': clock(e['emitted_at']),
                           'e': None, 'seq0': e['sequence'], 'seq1': None, 'outcome': 'running'})
        elif e['type'] == 'ParallelBranchCompleted':
            b = next((v for v in reversed(visits) if v.get('branchOf') is not None and v['node'] == e['branch'] and v['e'] is None), None)
            if b:
                b['e'] = clock(e['emitted_at']); b['seq1'] = e['sequence']; b['outcome'] = 'success' if e.get('success') else 'error'
    for n, v in enumerate(visits):
        v['i'] = n  # renumber: branches keep their own index; order in the list is handled by the renderer

    # --- child runs launched by a visit ---
    for e in events:
        if e['type'] == 'ChildRunStarted':
            v = next((v for v in reversed(visits) if v['node'] == e.get('parent_node_id') and v['seq0'] <= e['sequence']), None)
            if v:
                cm = read_json(os.path.join(ROOT, project, e['child_run_id'], 'run.json'), {})
                cev = read_lines(os.path.join(ROOT, project, e['child_run_id'], 'events.jsonl'))
                cnotes = [c.get('notes') for c in cev if c['type'] == 'StageCompleted' and c.get('notes')]
                v['child'] = {'id': e['child_run_id'][:8], 'flow': e.get('child_flow_name'), 's': clock(cm.get('started_at')),
                              'e': clock(cm.get('ended_at')), 'tokens': cm.get('token_usage') or 0,
                              'summary': short(cnotes[-1] if cnotes else '', 420) or None}
                if v['child']['summary']:
                    v['summary'] = v['child']['summary']

    # --- per-visit IO and transcript ---
    for v in visits:
        kind = next((n['kind'] for n in nodes if n['id'] == v['node']), 'agent')
        io = {'reads': [], 'writes': {}, 'systemWrites': 0}
        status, prompt_text, rows = None, None, []
        if v.get('stage') is not None:
            ex = os.path.join(d, 'logs', v['node'], 'executions', f"{v['stage']}-0")
            status = read_json(os.path.join(ex, 'status.json'))
            rows, prompt_text = rows_from_transcript(read_lines(os.path.join(ex, 'transcript.jsonl')))
            prompt_text = read_text(os.path.join(ex, 'prompt.md')) or prompt_text
            io['files'] = sorted(f for f in os.listdir(ex) if not f.startswith('.')) if os.path.isdir(ex) else []
        io['reads'], io['instr'] = prompt_parts(prompt_text) if kind == 'agent' else ([], None)
        if not io['instr'] and kind == 'agent':
            io['instr'] = short(node_cfg.get(v['node'], {}).get('prompt'), 700) or None
        if status:
            for k, val in (status.get('context_updates') or {}).items():
                if is_system(k):
                    io['systemWrites'] += 1
                else:
                    io['writes'][strip_ctx(k)] = None if val is None else short(val if isinstance(val, str) else json.dumps(val), 300)
            if not v.get('summary') and status.get('notes'):
                v['summary'] = short(status['notes'], 420)
        if kind == 'tool':
            cfg = node_cfg.get(v['node'], {})
            io['tool'] = short(cfg.get('tool.command') or cfg.get('command') or v['node'], 160)
            upd = (status or {}).get('context_updates') or {}
            out = upd.get('context.tool.output')
            io['output'] = (out[-600:] if isinstance(out, str) else None)
            io['exit'] = upd.get('context.tool.exit_code')
        v['io'] = io
        v['rows'] = trim_rows(rows)
        for k in ('seq0', 'seq1', 'stage'):
            v.pop(k, None)

    # --- run facts ---
    title_input = next((i for i in inputs if launch.get(i['key'])), None)
    first_value = short(launch.get(title_input['key']), 140) if title_input else None
    status = meta.get('status')
    failure = None
    if status == 'failed':
        msg = meta.get('last_error') or meta.get('outcome_reason_message') or ''
        failure = {'message': short(msg, 300), 'kind': 'infrastructure' if INFRA.search(msg) else 'flow',
                   'at': next((v['node'] for v in reversed(visits) if v.get('branchOf') is None), None)}
    if status in ('completed', 'failed', 'canceled'):  # a finished run can't still be running a visit
        for v in visits:
            if v['outcome'] == 'running':
                v['outcome'] = 'interrupted'
    question = None
    if replay_until:
        status = 'waiting'
        ev = read_lines(os.path.join(d, 'events.jsonl'))
        asked = next((e for e in ev if e['sequence'] >= replay_until and e['type'] == 'InterviewCompleted'), None)
        v = visits[-1]; v['outcome'] = 'waiting'; v['e'] = None
        ex = os.path.join(d, 'logs', v['node'], 'executions')
        seg = next((r['segment'] for p in sorted(os.listdir(ex)) for r in read_lines(os.path.join(ex, p, 'transcript.jsonl'))
                    if r.get('type') == 'segment_upsert' and r['segment'].get('kind') == 'request_user_input'), None) if os.path.isdir(ex) else None
        q = ((seg or {}).get('request_user_input') or {}).get('questions', [{}])[0]
        question = {'text': q.get('question') or (asked or {}).get('question'), 'header': q.get('header'), 'node': v['node'], 'visit': v['i'],
                    'options': [{'label': o['label'], 'description': o.get('description')} for o in q.get('options', [])],
                    'allowOther': bool(q.get('isOther')), 'askedAt': clock(seg['timestamp']) if seg and seg.get('timestamp') else None}
        # drop transcript rows after the question was asked
        rows = v['rows']; cut = next((k for k, r in enumerate(rows) if r[0] == 'ask'), None)
        v['rows'] = rows[:cut + 1] if cut is not None else rows
        v['summary'] = None; v['io']['writes'] = {}; v['io']['systemWrites'] = 0
    parent = None
    if meta.get('parent_run_id'):
        pd = os.path.join(ROOT, project, meta['parent_run_id'])
        pm = read_json(os.path.join(pd, 'run.json'), {})
        pev = read_lines(os.path.join(pd, 'events.jsonl'))
        started = [e for e in pev if e['type'] == 'StageStarted']
        cs = next((e for e in pev if e['type'] == 'ChildRunStarted' and e.get('child_run_id') == run_id), None)
        if cs:
            same = [e for e in started if e['node_id'] == cs['parent_node_id']]
            host = next((e for e in reversed(same) if e['sequence'] < cs['sequence']), None)
            parent = {'runId': meta['parent_run_id'], 'flow': _flow_title(pd, pm), 'node': cs['parent_node_id'],
                      'visitNo': same.index(host) + 1 if host else None, 'visits': len(same),
                      'title': _title(pd, pm)}
    result_md = read_text(os.path.join(d, 'result/result.md'), 1600) if status == 'completed' else None
    rj = read_json(os.path.join(d, 'result/result.json'), {}) or {}
    result_from = {'node': rj.get('source_node_id'), 'mode': rj.get('display_mode'), 'summarized': bool(rj.get('summary_enabled'))} if rj else None
    ctx = (read_json(os.path.join(d, 'checkpoint.json'), {}) or {}).get('context', {})
    trig = ctx.get('context.spark_trigger')
    trig = json.loads(trig) if isinstance(trig, str) else trig
    started_by = {'kind': 'trigger', 'name': trig.get('trigger_name')} if isinstance(trig, dict) and trig.get('trigger_name') else ({'kind': 'parent'} if meta.get('parent_run_id') else None)
    return {
        'id': run_id, 'note': note, 'flow': meta.get('flow_name'), 'flowTitle': flow.get('title') or meta.get('flow_name'),
        'goal': short(flow.get('goal'), 220), 'title': _title(d, meta), 'titleFrom': title_input['label'] if title_input else None,
        'inputs': [{'label': i['label'], 'key': strip_ctx(i['key']), 'value': short(launch.get(i['key']), 300)} for i in inputs if launch.get(i['key'])],
        'status': status, 'started': clock(meta.get('started_at')), 'ended': clock(meta.get('ended_at')) if status != 'waiting' else None,
        'date': ts(meta['started_at']).strftime('%b %-d') if meta.get('started_at') else None,
        'tokens': meta.get('token_usage') or 0, 'failure': failure, 'question': question, 'parent': parent,
        'commit': ctx.get('context.workspace.commit') or meta.get('git_commit'), 'branch': ctx.get('context.workspace.branch') or meta.get('git_branch'),
        'result': result_md, 'resultFrom': result_from, 'startedBy': started_by, 'nodes': nodes, 'edges': edges, 'visits': visits,
        'systemKeyCount': sum(1 for k in ctx if is_system(k)),
    }


def _flow_title(run_dir, meta):
    flow = read_json(os.path.join(run_dir, 'artifacts/flow/flow-definition.json'), {})
    return flow.get('title') or meta.get('flow_name')


def _title(run_dir, meta):
    """Generic run title: the first launch input the flow declares that was actually provided."""
    flow = read_json(os.path.join(run_dir, 'artifacts/flow/flow-definition.json'), {})
    launch = meta.get('launch_context') or {}
    first = next((i for i in flow.get('inputs') or [] if launch.get(i['key'])), None)
    return short(launch[first['key']], 140) if first else None


if __name__ == '__main__':
    out = [extract(*r) for r in RUNS]
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, 'runs.js'), 'w') as f:
        f.write('// Generated by extract.py from real run records. Do not edit by hand.\n')
        f.write('const RUN_DATA = ' + json.dumps(out, indent=1, ensure_ascii=False) + '\n')
    for r in out:
        print(f"{r['id'][:20]:20} {r['status']:9} visits={len(r['visits']):3} nodes={len(r['nodes']):2} title={r['title']!r:.70}")
