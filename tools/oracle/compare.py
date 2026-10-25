"""Compare `phpx optrees` output with optrees.php output. Exit 1 on any mismatch."""
import sys, collections
def load(p):
    d={}; cur=None
    for l in open(p):
        l=l.rstrip('\n')
        if l.startswith('== '): cur=l[3:]; d[cur]=[]
        else: d[cur].append(l)
    return d
a=load(sys.argv[1]); b=load(sys.argv[2])
bad=0; miss=collections.Counter(); extra=collections.Counter(); ex={}
for f,lines in a.items():
    if lines[:1]==['!! has errors']: continue
    ca=collections.Counter(lines); cb=collections.Counter(b.get(f,[]))
    if ca!=cb:
        bad+=1
        for k,v in (cb-ca).items(): miss[k]+=v; ex.setdefault(('m',k),f)
        for k,v in (ca-cb).items(): extra[k]+=v; ex.setdefault(('e',k),f)
print("files compared:", sum(1 for l in a.values() if l[:1]!=['!! has errors']), "mismatched:", bad)
print("--- only in php-parser (top):")
for k,v in miss.most_common(12): print(v, k[:150], '|', ex[('m',k)].split('vendor/')[-1])
print("--- only in phpx (top):")
for k,v in extra.most_common(12): print(v, k[:150], '|', ex[('e',k)].split('vendor/')[-1])
sys.exit(1 if bad else 0)
