'use strict';
let state, selected, action, invoker;
let selectionVersion=0, refreshVersion=0;
const $ = id => document.getElementById(id);
function node(tag, text, cls) { const el=document.createElement(tag); if(text!==undefined)el.textContent=text; if(cls)el.className=cls; return el; }
async function api(path, body) { const response=await fetch(path,body?{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)}:{}); const value=await response.json();if(!response.ok)throw Error(value.error||'Request failed');return value; }
function showError(error) {$('error').hidden=false;$('error').textContent=error.message;}
function details(title,value) {const el=node('details');el.append(node('summary',title),node('pre',typeof value==='string'?value:JSON.stringify(value,null,2)));return el;}
async function refresh() {
  const version=++refreshVersion;
  try {const loaded=await api('/api/status');if(version!==refreshVersion)return;state=loaded;$('error').hidden=true;const summary=$('summary');summary.replaceChildren();
    const reservations=Object.values(state.reservations);for(const text of [state.kind==='fixture'?'Fixture — no model-quality claim':'Research campaign',`Active: ${state.active.slice(0,12)}`,`Model requests reserved: ${reservations.length}`,`Reserved spend: $${(reservations.reduce((n,r)=>n+r.reserved,0)/1e6).toFixed(4)}`,`Confirmation: ${state.confirmation_spent?'consumed':'unused'}`,state.inflight?'Operation pending — inspect/recover in CLI':'No pending operation'])summary.append(node('p',text));
    const revisions=$('revisions');revisions.replaceChildren();for(const [key,entry] of Object.entries(state.lineage)){const button=node('button',`${entry.state} · ${key.slice(0,12)}`,'revision'+(selected===key?' selected':''));button.type='button';button.addEventListener('click',()=>select(key));revisions.append(button);}
    if(!selected||!state.lineage[selected])selected=state.active;await select(selected);
  } catch(error){if(version===refreshVersion)showError(error);}
}
async function select(key){
  const version=++selectionVersion, snapshot=state;
  selected=key;const entry=snapshot.lineage[key];try{const change=await api('/api/change?revision='+encodeURIComponent(key));if(version!==selectionVersion)return;const panel=$('review');panel.replaceChildren(node('h2',entry.parent?'Change review':'Baseline'),node('p',key,'hash'),node('p',`State: ${entry.state}`));
    if(change.proposal){panel.append(node('h3','Prediction'),node('p',change.proposal.hypothesis),node('h3','Mechanism'),node('p',change.proposal.mechanism),node('h3','Risks'),node('p',change.proposal.risks));}
    if(change.authority_paths?.length)panel.append(node('p','Authority-sensitive changes require careful external review: '+change.authority_paths.join(', '),'warn'));
    panel.append(details('Candidate diff',change.patch),details('Settings and memory',{configuration:change.configuration,memory:change.memory}));
    let verdict;if(snapshot.verdicts[key]){verdict=await api('/api/record?digest='+encodeURIComponent(snapshot.verdicts[key]));if(version!==selectionVersion)return;panel.append(node('h3',`Verdict: ${verdict.verdict}`),node('p',verdict.claim),details('Matched comparisons and evidence',verdict));}
    if(entry.gates)panel.append(details('Independent gate records',entry.gates));
    const controls=node('div',undefined,'actions');if(snapshot.pending&&snapshot.verdicts[key]===snapshot.pending){const approve=node('button','Review and activate in lab','primary');approve.disabled=!!snapshot.inflight;approve.title=approve.disabled?'Recover the pending operation first.':'Opens a confirmation; does not install Pactrail.';approve.addEventListener('click',()=>confirmAction('approve',key,approve,snapshot));controls.append(approve);}
    if(entry.parent&&['active','ancestor'].includes(entry.state)){const undo=node('button','Undo this change');undo.disabled=!!snapshot.inflight;undo.title=undo.disabled?'Recover the pending operation first.':'Restores parent code, binary, settings and memory; retires descendants.';undo.addEventListener('click',()=>confirmAction('undo',key,undo,snapshot));controls.append(undo);}panel.append(controls);
    for(const button of $('revisions').children)button.classList.toggle('selected',button.textContent.endsWith(key.slice(0,12)));
  }catch(error){if(version===selectionVersion)showError(error);}
}
function confirmAction(kind,key,button,snapshot){action={kind,key,head:snapshot.head,verdict:snapshot.verdicts[key]};invoker=button;$('ack').checked=false;$('ack-row').hidden=kind==='undo';$('execute').disabled=kind==='approve';$('confirm-title').textContent=kind==='approve'?'Activate this exact candidate in the lab?':'Undo this change and its descendants?';$('confirm-copy').textContent=kind==='approve'?'Your source repository and installed binary are not changed. The accepted snapshot becomes the next lab parent.':'The complete parent snapshot is restored. Retired descendants cannot be activated. Existing evidence is retained.';$('execute').textContent=kind==='approve'?'Activate in lab':'Undo change';$('confirm').showModal();$('cancel').focus();}
$('ack').addEventListener('change',()=>{$('execute').disabled=!$('ack').checked;});
$('confirm').addEventListener('close',()=>{(invoker?.isConnected?invoker:$('refresh')).focus();});
$('execute').addEventListener('click',async()=>{const button=$('execute');button.disabled=true;$('cancel').disabled=true;const command_id='review-'+crypto.randomUUID().replaceAll('-','');try{const body=action.kind==='approve'?{command_id,head:action.head,verdict:action.verdict,acknowledgment:$('ack').checked}:{command_id,head:action.head,revision:action.key};await api('/api/'+action.kind,body);$('confirm').close();$('notice').textContent=action.kind==='approve'?'Candidate activated in the lab. Production remains unchanged.':'Change undone. Parent snapshot restored.';$('live').textContent=$('notice').textContent;selected=null;await refresh();}catch(error){showError(error);$('confirm').close();}finally{button.disabled=false;$('cancel').disabled=false;}});
$('confirm').addEventListener('cancel',event=>{if($('cancel').disabled)event.preventDefault();});
$('refresh').addEventListener('click',refresh);refresh();
