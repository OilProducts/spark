"""Derive mockup data from real Spark missions. Writes missions.js next to this file.

Two missions are also replayed at an earlier moment, so the mockups can show the
running and needs-you states with real content.
"""
import collections, json, os, re
from datetime import datetime, timezone

HOME = os.path.expanduser('~/Library/Application Support/dev.spark.desktop/spark')
PROJ = os.path.join(HOME, 'workspace/projects/spark-9301d20d6c95')
RUNS = os.path.join(HOME, 'attractor/runs/spark-9301d20d6c95')

def ts(value):
    """Mission timestamps are 'YYYY-MM-DD H:MM:SS.frac +00:00:00'; transcripts use ISO."""
    if not value:
        return None
    m = re.match(r'(\d{4}-\d{2}-\d{2})[ T](\d{1,2}):(\d{2}):(\d{2})', value)
    if not m:
        return None
    return f'{m.group(1)}T{int(m.group(2)):02d}:{m.group(3)}:{m.group(4)}Z'

def run_facts(run_id):
    path = os.path.join(RUNS, run_id, 'run.json')
    if not os.path.exists(path):
        return {}
    r = json.load(open(path))
    usage = r.get('token_usage_breakdown') or {}
    return {'title': r.get('title'), 'runStatus': r.get('status'), 'started': r.get('started_at'),
            'ended': r.get('ended_at'), 'tokens': usage.get('total_tokens') or r.get('token_usage'),
            'commit': r.get('git_commit')}

def beats(conversation_id):
    """The conversation as beats: what came in (objective, run event, question, your reply) and each agent turn."""
    root = os.path.join(PROJ, 'conversations', conversation_id)
    path = os.path.join(root, 'transcript.jsonl')
    if not os.path.exists(path):
        return []
    turns, segs = {}, {}
    for line in open(path):
        r = json.loads(line)
        if r['type'] == 'turn_upsert':
            turns[r['turn']['id']] = r['turn']
        elif r['type'] == 'segment_upsert':
            segs[r['segment']['id']] = r['segment']
    launches = {}
    lp = os.path.join(root, 'artifacts', 'flow-launches.json')
    if os.path.exists(lp):
        for a in json.load(open(lp)).get('flow_launches', []) if isinstance(json.load(open(lp)), dict) else json.load(open(lp)):
            launches[a.get('id')] = a.get('run_id')
    by_turn = collections.defaultdict(list)
    for s in segs.values():
        by_turn[s['turn_id']].append(s)
    out = []
    for t in sorted(turns.values(), key=lambda t: (t.get('created_at') or t.get('timestamp') or '')):
        ss = sorted(by_turn[t['id']], key=lambda s: s['order'])
        at = ts(t.get('created_at') or t.get('timestamp'))
        text = (t.get('content') or '').strip()
        if t['role'] == 'user':
            if text.startswith('Objective:'):
                out.append({'kind': 'objective', 'at': at})
            elif text.startswith('Run question:'):
                q = json.loads(text[len('Run question:'):])
                out.append({'kind': 'question', 'at': at, 'prompt': q['prompt'], 'options': [o['label'] for o in q['options']],
                            'flow': q.get('flow_name'), 'run': q.get('root_run_id')})
            elif text.startswith('Run '):
                m = re.match(r'Run (\S+) \(([^,]+), "(.*)"\) ended (\w+)', text)
                out.append({'kind': 'run_event', 'at': at, 'run': m.group(1) if m else None,
                            'status': m.group(4) if m else None, 'text': text})
            else:
                out.append({'kind': 'you', 'at': at, 'text': re.sub(r'^User:\s*', '', text)})
        elif t['role'] == 'assistant':
            commentary = [s.get('content', '') for s in ss if s['kind'] == 'assistant_message' and s.get('phase') != 'final_answer']
            tools = [(s.get('tool_call') or {}).get('command') or (s.get('tool_call') or {}).get('title') for s in ss if s['kind'] == 'tool_call']
            launched = [launches.get(s.get('artifact_id')) for s in ss if s['kind'] == 'flow_launch']
            out.append({'kind': 'agent', 'at': at, 'final': text, 'commentary': commentary,
                        'tools': [x for x in tools if x][:12], 'toolCount': len(tools), 'launched': [x for x in launched if x]})
        elif t['role'] == 'system':
            out.append({'kind': 'closed', 'at': at, 'text': text})
    return out

def mission(path):
    d = json.load(open(path))
    runs = [{**r, 'launched_at': ts(r.get('launched_at')), **run_facts(r['run_id'])} for r in d.get('runs') or []]
    closed = d.get('closed')
    status = 'closed' if closed else ('draft' if not d.get('started_at') else 'running')
    return {
        'id': d['id'], 'title': d['fields']['title'], 'objective': d['fields'].get('description') or '',
        'playbook': (d.get('playbook') or {}).get('title') or d['fields'].get('playbook'),
        'playbookText': (d.get('playbook') or {}).get('text'),
        'budget': d['fields'].get('budget'), 'archived': d['fields'].get('archived'),
        'status': status, 'created': ts(d.get('created_at')), 'started': ts(d.get('started_at')), 'updated': ts(d.get('updated_at')),
        'closed': closed and {**closed, 'at': ts(closed.get('at'))},
        'activity': [{'at': ts(a.get('at')), 'actor': a.get('actor'), 'note': a.get('note')} for a in d.get('activity', [])],
        'runs': runs, 'beats': beats(d.get('conversation_id') or d['id']),
    }

def replay(m, keep_beats, status, note, run_overrides=None, extra=None):
    """The mission as it stood partway through: its first beats, runs launched by then, not closed."""
    beats = m['beats'][:keep_beats]
    last = beats[-1]['at']
    launched = {run for b in beats for run in b.get('launched', [])}
    runs = [dict(r) for r in m['runs'] if r['run_id'] in launched]
    for r in runs:
        r.update((run_overrides or {}).get(r['run_id'], {}))
    return {**m, 'id': m['id'] + '-replay', 'status': status, 'closed': None, 'beats': beats, 'runs': runs,
            'updated': last, 'replay': note, 'archived': False, **(extra or {})}

ms = {os.path.basename(p)[:-5]: mission(os.path.join(PROJ, 'missions', p))
      for p in os.listdir(os.path.join(PROJ, 'missions')) if p.endswith('.json')}
question = ms['mission-a15629d0-978b-453d-a1ca-8507b1942e17']
load = ms['mission-c5505392-f161-43f0-ac0d-12e8572513dd']
out = [
    # Needs you: the question test just after the agent asked you whether to ship.
    replay(question, 6, 'needs_you', 'Replayed at 1:20 AM Sep 28, when the agent asked you whether to ship',
           {'run-18d9576054ba5b78': {'status': 'waiting', 'runStatus': 'waiting', 'ended': None}},
           {'ask': {'prompt': 'Ship it?', 'options': ['Hold', 'Ship it'], 'from': 'testing/question-parent.yaml',
                    'agentSays': 'Should the run **Hold** or **Ship it**? Your objective reserves this decision for you.'}}),
    # Running: the load-test mission while its independent review was underway.
    replay(load, 4, 'running', 'Replayed at 6:19 PM Sep 27, while the independent review ran',
           {'run-18d94d8c7c5cfd80': {'status': 'running', 'runStatus': 'running', 'ended': None}}),
    ms['mission-1934529e-9100-4a80-9ecd-475df28eccfa'],
    load,
    ms['task-ff1120d6-5bab-495a-afa5-24e0bad78b7e'],
    {**question, 'archived': True},
]
here = os.path.dirname(os.path.abspath(__file__))
with open(os.path.join(here, 'missions.js'), 'w') as f:
    f.write('// Generated by extract.py from real Spark missions. Do not edit by hand.\nconst MISSIONS = ')
    json.dump(out, f, indent=1, ensure_ascii=False)
    f.write(';\n')
print([(m['title'][:40], m['status'], len(m['beats']), len(m['runs'])) for m in out])
