const t = process.hrtime.bigint();
const words = []; for (let i = 0; i < 5000; i++) words.push('w' + (i * 7919 % 1500));
let tot = 0;
for (let r = 0; r < 60; r++) {
  const count = new Map();
  for (const w of words) count.set(w, (count.get(w) ?? 0) + 1);
  const byLen = new Map();
  for (const [w, c] of count) { const l = w.length; if (!byLen.has(l)) byLen.set(l, new Map()); byLen.get(l).set(w, c); }
  const sorted = [...count.entries()].sort((a, b) => b[1] - a[1]);
  let topSum = 0; for (const [, c] of sorted.slice(0, 10)) topSum += c;
  const rows = [];
  for (let i = 0; i < 2000; i++) rows.push({ id: i, name: 'n' + i, score: i % 97, tags: { a: i % 3, b: i % 5 } });
  let sum = 0; for (const row of rows) { if (row.tags.a !== undefined && row.score > 50) sum += row.tags.b + row.id; }
  tot = (tot + count.size + byLen.size + topSum + sum) % 1000000007;
}
console.log(`assoc ${Math.round(Number(process.hrtime.bigint() - t) / 1e6)} ms ${tot}`);
