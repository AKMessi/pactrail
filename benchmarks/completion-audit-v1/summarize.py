#!/usr/bin/env python3
"""Summarize retained trials without treating absent results as failures or zeros."""
import json
import pathlib
import sys
from run import ARMS, write

def summarize(folder):
    protocol=json.loads((folder/'protocol.json').read_text())
    results=json.loads((folder/'results.json').read_text()) if (folder/'results.json').exists() else []
    arms={}
    for arm in ARMS:
        rows=[r for r in results if r['arm']==arm]
        graded=[r for r in rows if r.get('functional') is not None]
        usages=[u for r in rows for u in r.get('usage',[])]
        arms[arm]={
            'planned':sum(s['arm']==arm for s in protocol['order']),
            'retained':len(rows),'graded':len(graded),
            'functional_passes':sum(r['functional']['passed'] for r in graded) if graded else None,
            'strict_passes':sum(r['strict_passed'] for r in rows) if rows else None,
            'requests':sum(r['requests'] for r in rows),
            'token_usage':{key:{'reported_sum':sum(u[key] for u in usages if isinstance(u,dict) and isinstance(u.get(key),int)) if any(isinstance(u,dict) and isinstance(u.get(key),int) for u in usages) else None,
                                'reported_requests':sum(isinstance(u,dict) and isinstance(u.get(key),int) for u in usages),
                                'total_requests':len(usages)} for key in ['prompt_tokens','completion_tokens','total_tokens']}}
    by_key={(r['case'],r['repetition'],r['arm']):r for r in results}
    paired={}
    for arm in ['pactrail-baseline','opencode','mini']:
        counts={'pairs':0,'audit_wins':0,'ties':0,'audit_losses':0}
        for row in results:
            if row['arm']!='pactrail-audit' or row.get('functional') is None:continue
            other=by_key.get((row['case'],row['repetition'],arm))
            if other is None or other.get('functional') is None:continue
            a=row['functional']['passed'];b=other['functional']['passed'];counts['pairs']+=1
            counts['ties' if a==b else 'audit_wins' if a else 'audit_losses']+=1
        paired[arm]=counts
    return {'planned_trials':len(protocol['order']),'retained_trials':len(results),'pending_trials':len(protocol['order'])-len(results),
            'arms':arms,'paired_functional':paired,
            'limitations':'Six synthetic cases; repeated samples share cases. No global superiority, true streaming latency, or monetary savings claim.'}

if __name__=='__main__':
    folder=pathlib.Path(sys.argv[1]);result=summarize(folder);write(folder/'summary.json',result)
    print(json.dumps(result,indent=2))
