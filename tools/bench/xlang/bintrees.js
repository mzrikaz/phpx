class Node { constructor(l = null, r = null) { this.l = l; this.r = r; } }
function make(d) { return d === 0 ? new Node() : new Node(make(d - 1), make(d - 1)); }
function check(n) { return n.l === null ? 1 : 1 + check(n.l) + check(n.r); }
const max = 16, t = process.hrtime.bigint(), out = [];
out.push(check(make(max + 1)));
const long = make(max);
for (let d = 4; d <= max; d += 2) { const it = 1 << (max - d + 4); let c = 0; for (let i = 0; i < it; i++) c += check(make(d)); out.push(c); }
out.push(check(long));
console.log(`bintrees ${Math.round(Number(process.hrtime.bigint() - t) / 1e6)} ms ${out.join(',')}`);
