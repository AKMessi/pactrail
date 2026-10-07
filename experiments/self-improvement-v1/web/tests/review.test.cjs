'use strict';
// Test control/data binding with a minimal DOM, not browser layout or accessibility.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
function element(){return {children:[],listeners:{},classList:{toggle(){}},
  append(...nodes){this.children.push(...nodes);},replaceChildren(...nodes){this.children=nodes;},
  addEventListener(name,handler){this.listeners[name]=handler;},focus(){this.focused=true;},
  showModal(){this.open=true;},close(){this.open=false;}};}
const elements=new Map();
const context=vm.createContext({document:{getElementById(id){if(!elements.has(id))elements.set(id,element());return elements.get(id);},createElement:element},
  fetch:()=>new Promise(()=>{}),crypto:{randomUUID:()=> 'fixture'},console});
vm.runInContext(fs.readFileSync(require('node:path').join(__dirname,'../app.js'),'utf8'),context);
vm.runInContext(`
  state={head:'new-head',pending:'new-verdict'};
  confirmAction('approve','reviewed-candidate',{},
    {head:'reviewed-head',verdicts:{'reviewed-candidate':'reviewed-verdict'}});
`,context);
const action=vm.runInContext('action',context);
assert.equal(action.head,'reviewed-head');
assert.equal(action.key,'reviewed-candidate');
assert.equal(action.verdict,'reviewed-verdict');
assert.equal(elements.get('cancel').focused,true);
assert.equal(elements.get('execute').disabled,true);
const pending=new Map();
context.fetch=path=>new Promise(resolve=>pending.set(path,resolve));
vm.runInContext(`state={head:'head',lineage:{first:{parent:'parent',state:'candidate'},second:{parent:'parent',state:'candidate'}},verdicts:{},pending:null};`,context);
const first=vm.runInContext("select('first')",context);
const second=vm.runInContext("select('second')",context);
function resolve(key){pending.get('/api/change?revision='+key)({ok:true,json:async()=>({patch:key,configuration:{},memory:[]})});}
(async()=>{
  resolve('second'); await second;
  const before=elements.get('review').children;
  assert.equal(before[1].textContent,'second');
  resolve('first'); await first;
  assert.equal(elements.get('review').children,before);
  assert.equal(vm.runInContext('selected',context),'second');
  console.log('PASS: exact displayed approval binding and out-of-order selection');
})().catch(error=>{console.error(error);process.exitCode=1;});
