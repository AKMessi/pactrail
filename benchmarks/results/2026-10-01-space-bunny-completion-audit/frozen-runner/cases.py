"""Frozen controlled completeness tasks. Graders/gold never enter agent workspaces."""
CASES = [
    dict(id="paired-clamping", prompt="Fix numeric range clipping consistently in scalar.clip and batches.clip_many. Both endpoints are inclusive; reversed bounds must raise ValueError, including for an empty batch. Do not mutate the input sequence. Preserve input order and numeric values within range.",
         files={"scalar.py":"def clip(value, lower, upper):\n    return min(value, upper)\n", "batches.py":"def clip_many(values, lower, upper):\n    return [max(v, lower) for v in values]\n"},
         gold={"scalar.py":"def clip(value, lower, upper):\n    if lower > upper: raise ValueError('bounds')\n    return min(max(value, lower), upper)\n", "batches.py":"from scalar import clip\ndef clip_many(values, lower, upper):\n    if lower > upper: raise ValueError('bounds')\n    return [clip(v, lower, upper) for v in values]\n"},
         grader="""from scalar import clip
from batches import clip_many
for lower,upper in [(-3,4),(0,0),(1.5,9.5)]:
 for v in [-20,-3,0,2,9.5,30]: assert clip(v,lower,upper)==min(max(v,lower),upper)
values=[-5,0,4,20]; before=values.copy()
assert clip_many(values,0,5)==[0,0,4,5] and values==before
for fn,args in [(clip,(0,4,1)),(clip_many,([],4,1))]:
 try: fn(*args)
 except ValueError: pass
 else: raise AssertionError('reversed bounds accepted')
"""),
    dict(id="unicode-key-collisions",prompt="Fix case-insensitive key handling across keys.canonical and catalog.Catalog. Canonical keys must use Unicode NFC normalization and casefold, not lower. put replaces the value for equivalent keys; get and remove accept any equivalent spelling. Missing keys must still raise KeyError. Preserve the public method signatures.",
         files={"keys.py":"def canonical(key):\n    return key.lower()\n", "catalog.py":"from keys import canonical\nclass Catalog:\n    def __init__(self): self.values = {}\n    def put(self, key, value): self.values[canonical(key)] = value\n    def get(self, key): return self.values[key.lower()]\n    def remove(self, key): return self.values.pop(key.lower())\n"},
         gold={"keys.py":"import unicodedata\ndef canonical(key):\n    return unicodedata.normalize('NFC', key).casefold()\n", "catalog.py":"from keys import canonical\nclass Catalog:\n    def __init__(self): self.values = {}\n    def put(self, key, value): self.values[canonical(key)] = value\n    def get(self, key): return self.values[canonical(key)]\n    def remove(self, key): return self.values.pop(canonical(key))\n"},
         grader="""from keys import canonical
from catalog import Catalog
assert canonical('Straße')=='strasse'
assert canonical('E\\u0301')==canonical('É')
c=Catalog(); c.put('Straße',1); assert c.get('STRASSE')==1
c.put('STRASSE',2); assert len(c.values)==1 and c.remove('straße')==2
c.put('E\\u0301',3); assert c.get('É')==3 and c.remove('é')==3
try: c.get('missing')
except KeyError: pass
else: raise AssertionError('missing key')
"""),
    dict(id="decimal-invoice-rounding",prompt="Fix money.cents and invoices.total_cents. Convert input values using Decimal(str(value)); round each line independently to integral cents with ROUND_HALF_UP, including negative ties, then sum. Do not round only the invoice total. Preserve empty total 0 and the integer return types. No binary floating-point intermediate should determine rounding.",
         files={"money.py":"def cents(amount):\n    return round(float(amount) * 100)\n", "invoices.py":"def total_cents(amounts):\n    return round(sum(float(a) for a in amounts) * 100)\n"},
         gold={"money.py":"from decimal import Decimal, ROUND_HALF_UP\ndef cents(amount):\n    return int((Decimal(str(amount))*100).quantize(Decimal('1'), rounding=ROUND_HALF_UP))\n", "invoices.py":"from money import cents\ndef total_cents(amounts):\n    return sum(cents(a) for a in amounts)\n"},
         grader="""from money import cents
from invoices import total_cents
from decimal import Decimal
for v,expected in [('1.005',101),('-1.005',-101),('2.675',268),('0.005',1),('-0.005',-1),('0',0),('999.99',99999)]: assert cents(v)==expected,(v,cents(v))
assert total_cents(['0.005','0.005'])==2
assert total_cents([Decimal('1.005'),'-0.005'])==100
assert total_cents([])==0 and isinstance(total_cents([]),int)
"""),
    dict(id="pagination-boundary",prompt="Fix pages.page and api.collect. page(items,cursor,size) returns a list slice and the next integer cursor, or None when exhausted. Reject size<=0 and cursor<0 or cursor>len(items) with ValueError. Cursor len(items) returns ([],None). collect must gather every item exactly once, preserve order, handle empty input, and not mutate it. The first cursor is 0, not a truthiness-based sentinel.",
         files={"pages.py":"def page(items, cursor, size):\n    end = cursor + size\n    return list(items[cursor:end]), end if end <= len(items) else None\n", "api.py":"from pages import page\ndef collect(items, size):\n    cursor = 0\n    result = []\n    while cursor:\n        rows, cursor = page(items, cursor, size)\n        result.extend(rows)\n    return result\n"},
         gold={"pages.py":"def page(items, cursor, size):\n    if size <= 0 or cursor < 0 or cursor > len(items): raise ValueError('page bounds')\n    end=min(cursor+size,len(items))\n    return list(items[cursor:end]), end if end < len(items) else None\n", "api.py":"from pages import page\ndef collect(items, size):\n    cursor = 0\n    result = []\n    while cursor is not None:\n        rows, cursor = page(items, cursor, size)\n        result.extend(rows)\n    return result\n"},
         grader="""from pages import page
from api import collect
for n in range(9):
 values=list(range(n)); before=values.copy()
 for size in [1,2,4,10]: assert collect(values,size)==values
 assert values==before and page(values,n,2)==([],None)
assert page([1,2],0,2)==([1,2],None)
for args in [([],0,0),([],0,-1),([1],-1,1),([1],2,1)]:
 try: page(*args)
 except ValueError: pass
 else: raise AssertionError('invalid page accepted')
"""),
    dict(id="expiry-timezone-consistency",prompt="Fix clock.expired and cache.Cache. Naive datetime values mean UTC; aware values must compare as instants. Entries expire at now>=expires_at, not just after. put accepts a value and absolute expiry; get(key,now) must use the shared expiry semantics, delete expired entries and raise KeyError; live and missing entries keep their usual behavior. Do not read the wall clock implicitly.",
         files={"clock.py":"def expired(expires_at, now):\n    return now > expires_at\n", "cache.py":"class Cache:\n    def __init__(self): self.entries={}\n    def put(self,key,value,expires_at): self.entries[key]=(value,expires_at)\n    def get(self,key,now):\n        value,expiry=self.entries[key]\n        if now > expiry: raise KeyError(key)\n        return value\n"},
         gold={"clock.py":"from datetime import timezone\ndef expired(expires_at, now):\n    def aware(d): return d.replace(tzinfo=timezone.utc) if d.tzinfo is None else d\n    return aware(now)>=aware(expires_at)\n", "cache.py":"from clock import expired\nclass Cache:\n    def __init__(self): self.entries={}\n    def put(self,key,value,expires_at): self.entries[key]=(value,expires_at)\n    def get(self,key,now):\n        value,expiry=self.entries[key]\n        if expired(expiry,now):\n            del self.entries[key]\n            raise KeyError(key)\n        return value\n"},
         grader="""from datetime import datetime,timezone,timedelta
from clock import expired
from cache import Cache
expiry=datetime(2026,1,1,12); equal=datetime(2026,1,1,14,tzinfo=timezone(timedelta(hours=2)))
assert expired(expiry,equal) and expired(equal,expiry)
assert not expired(expiry,datetime(2026,1,1,11,59,tzinfo=timezone.utc))
c=Cache();c.put('a',7,expiry); assert c.get('a',expiry-timedelta(seconds=1))==7
try: c.get('a',equal)
except KeyError: pass
else: raise AssertionError('expiry boundary')
assert 'a' not in c.entries
"""),
    dict(id="stable-topological-order",prompt="Fix graph.order and build.plan. graph.order takes a mapping from node to prerequisite node names and returns every declared node exactly once, prerequisites before dependents. Choose the lexicographically smallest currently-ready node each step for deterministic results. Unknown prerequisites and cycles must raise ValueError. build.plan(graph,requested) returns the ordered transitive prerequisite closure of requested nodes; unknown requested nodes raise ValueError. Do not mutate the graph or prerequisite lists.",
         files={"graph.py":"def order(graph):\n    return sorted(graph)\n", "build.py":"from graph import order\ndef plan(graph,requested):\n    return [n for n in order(graph) if n in requested]\n"},
         gold={"graph.py":"import heapq\ndef order(graph):\n    pending={n:set(ds) for n,ds in graph.items()}\n    if any(d not in graph for ds in pending.values() for d in ds): raise ValueError('unknown')\n    ready=[n for n,ds in pending.items() if not ds];heapq.heapify(ready);result=[]\n    while ready:\n        n=heapq.heappop(ready);result.append(n);del pending[n]\n        for k,ds in pending.items():\n            if n in ds:\n                ds.remove(n)\n                if not ds: heapq.heappush(ready,k)\n    if pending: raise ValueError('cycle')\n    return result\n", "build.py":"from graph import order\ndef plan(graph,requested):\n    sequence=order(graph);needed=set();stack=list(requested)\n    while stack:\n        n=stack.pop()\n        if n not in graph: raise ValueError('unknown requested')\n        if n not in needed:\n            needed.add(n);stack.extend(graph[n])\n    return [n for n in sequence if n in needed]\n"},
         grader="""from graph import order
from build import plan
import copy
g={'a':['z'],'z':[],'b':[],'c':['a','b']};before=copy.deepcopy(g)
assert order(g)==['b','z','a','c'] and plan(g,['c'])==['b','z','a','c']
assert plan(g,['a'])==['z','a'] and plan(g,[])==[] and g==before
assert order({})==[]
for bad in [{'a':['missing']},{'a':['b'],'b':['a']},{'a':['a']}]:
 try: order(bad)
 except ValueError: pass
 else: raise AssertionError('invalid graph')
try: plan(g,['missing'])
except ValueError: pass
else: raise AssertionError('unknown requested')
"""),
]
