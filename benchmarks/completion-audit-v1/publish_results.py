#!/usr/bin/env python3
"""Publish completed results with ephemeral proxy credentials redacted."""
import argparse
import hashlib
import json
import pathlib
import statistics
from run import ARMS, write
from summarize import summarize

LABELS={'pactrail-baseline':'Pactrail before upgrade','pactrail-audit':'Pactrail with audit',
        'opencode':'OpenCode 1.18.34','mini':'mini-SWE-agent 2.4.6'}

def publish(source,destination,frozen_source=None):
    protocol=json.loads((source/'protocol.json').read_text())
    results=json.loads((source/'results.json').read_text())
    if len(results)!=len(protocol['order']):
        raise RuntimeError('Refusing to publish a completed report for an incomplete experiment')
    if destination.exists(): raise RuntimeError('Refusing to overwrite published results')
    by_spec={(r['case'],r['arm'],r['repetition']):r for r in results}
    if len(by_spec)!=len(results): raise RuntimeError('Duplicate trial identity')
    for spec in protocol['order']:
        if (spec['case'],spec['arm'],spec['repetition']) not in by_spec: raise RuntimeError('Missing frozen trial')
    summary=summarize(source)
    secrets=set()
    for path in (source/'raw').glob('*/trajectory.json'):
        data=json.loads(path.read_text())
        value=data.get('info',{}).get('config',{}).get('model',{}).get('model_kwargs',{}).get('api_key')
        if isinstance(value,str) and value:secrets.add(value)
    # The real provider key must never occur in artifacts. Keep it outside this
    # script: the orchestrator's sanitized agent environments never include it.
    destination.mkdir(parents=True)
    manifest=[]
    for path in sorted((source/'raw').rglob('*')):
        if not path.is_file():continue
        original=path.read_bytes();text=original.decode()
        for value in secrets:text=text.replace(value,'<ephemeral-proxy-key-redacted>')
        target=destination/path.relative_to(source);target.parent.mkdir(parents=True,exist_ok=True)
        published=text.encode();target.write_bytes(published)
        manifest.append({'path':str(path.relative_to(source)),
                         'original_sha256':hashlib.sha256(original).hexdigest(),
                         'published_sha256':hashlib.sha256(published).hexdigest(),
                         'redacted':original!=published})
    frozen_source=frozen_source or (source/'frozen-runner' if (source/'frozen-runner').is_dir() else pathlib.Path(__file__).resolve().parent)
    for name,sha in protocol['files'].items():
        path=frozen_source/name
        raw=path.read_bytes()
        if hashlib.sha256(raw).hexdigest()!=sha: raise RuntimeError('Frozen runner source mismatch: '+name)
        target=destination/'frozen-runner'/name
        target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(raw)
    write(destination/'artifact-manifest.json',manifest)
    write(destination/'protocol.json',protocol);write(destination/'results.json',results)
    write(destination/'summary.json',summary)
    rows=['# Space Bunny Alpha: completion audit comparison','',
          f"Completed {len(results)} frozen trials: {len(set(r['case'] for r in results))} synthetic Python cases, {len(set(r['arm'] for r in results))} harness arms, {len(set(r['repetition'] for r in results))} repetitions.",
          'This measures this model on this small suite; it does not establish universal harness superiority.','',
          '## Aggregate outcomes','',
          '| Harness | Functional passes / graded | Recorded strict passes / retained | Admitted model requests | Reported tokens | Median agent seconds |',
          '| --- | ---: | ---: | ---: | ---: | ---: |']
    for arm in ARMS:
        a=summary['arms'][arm];r=[r for r in results if r['arm']==arm]
        u=a['token_usage']['total_tokens'];tokens='—' if u['reported_sum'] is None else f"{u['reported_sum']:,}"
        if u['reported_requests']<u['total_requests'] and tokens!='—':tokens='≥ '+tokens
        times=[x['agent_seconds'] for x in r if x.get('agent_seconds') is not None]
        seconds=f'{statistics.median(times):.1f}' if times else '—'
        fp='—' if a['functional_passes'] is None else str(a['functional_passes'])
        sp='—' if a['strict_passes'] is None else str(a['strict_passes'])
        rows.append(f"| {LABELS[arm]} | {fp} / {a['graded']} | {sp} / {a['retained']} | {a['requests']} | {tokens} | {seconds} |")
    rows+=['','Functional grading examines the candidate using external behavioral assertions.',
           'Recorded strict success requires clean exit, no timeout, and the frozen cleanliness check.',
           'Pactrail strict success also checks unchanged original production-file bytes, verified trace, successful apply,',
           'and matching production-file bytes after apply. These extra assurance conditions differ',
           'from the external harness checks; use functional outcomes for the common comparison.','',
           '## Paired audit comparison','',
           '| Comparator | Paired graded trials | Audit wins | Ties | Audit losses |',
           '| --- | ---: | ---: | ---: | ---: |']
    for arm,p in summary['paired_functional'].items():
        rows.append(f"| {LABELS[arm]} | {p['pairs']} | {p['audit_wins']} | {p['ties']} | {p['audit_losses']} |")
    rows+=['','## Cases','', '| Case | Before | Audit | OpenCode | mini |', '| --- | --- | --- | --- | --- |']
    for case in dict.fromkeys(s['case'] for s in protocol['order']):
        cells=[]
        for arm in ARMS:
            values=[]
            for r in sorted((x for x in results if x['case']==case and x['arm']==arm),key=lambda x:x['repetition']):
                values.append('ungraded' if r.get('functional') is None else 'pass' if r['functional']['passed'] else 'fail')
            cells.append(', '.join(values))
        rows.append('| '+case+' | '+' | '.join(cells)+' |')
    rows+=['','## Limits and reproducibility','',
           '- Every scored trial is retained; no replacement samples or post-result parameter tuning.',
           '- Frozen v1 cleanliness checks missed newly added empty files. Strict scores are as recorded, not a complete file-inventory audit; functional grading is unaffected. The runner is corrected for future experiments.',
           '- The request count is admitted upstream attempts. Local requests rejected after the cap are not counted in this ledger.',
           '- The frozen console progress label could show the last changed filename. Result identities and frozen order use JSON, which are unaffected.',
           '- Reproduce this scored version from commit 09e4ba6. Original runner sources are archived under frozen-runner; the current runner may contain subsequent fixes.',
           '- Six synthetic cases are not independent real repository issues; repeated trials share cases.',
           '- Exact free model, temperature 0, low reasoning, 8,192 output tokens, 12 HTTP attempts, 300-second agent deadline.',
           '- Common input bound is serialized message bytes, not equal tokenizer context windows.',
           '- Harness prompts, tools, recovery, and stopping differ. Native local commands are allowed.',
           '- Buffered upstream responses cannot measure true streaming latency.',
           '- API pricing was zero; this cannot prove monetary savings. Request usage coverage is in summary.json.',
           '- Agent seconds exclude external grading and Pactrail apply. Provider errors are retained per trial.',
           '- Original and published artifact hashes are in artifact-manifest.json; only ephemeral proxy credentials are redacted.',
           '- Runner and task hashes, pinned versions, original implementation hashes, and frozen order are in protocol.json.',
           '- Official comparator sources: https://opencode.ai/docs/cli and https://mini-swe-agent.com/latest/reference/run/mini/.',
           '', 'The policy remains opt-in. Default rollout requires broader independent issues and cost/outcome evidence.','']
    (destination/'README.md').write_text('\n'.join(rows))

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('source',type=pathlib.Path);parser.add_argument('destination',type=pathlib.Path)
    parser.add_argument('--frozen-source-dir',type=pathlib.Path,help='Original frozen runner directory when the current runner has changed.')
    args=parser.parse_args();publish(args.source,args.destination,args.frozen_source_dir)
