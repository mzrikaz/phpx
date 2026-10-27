const t = process.hrtime.bigint(); let tot = 0;
const data = []; for (let i = 0; i < 3000; i++) data.push(Number((BigInt(i) * 2654435761n) % 10007n));
for (let r = 0; r < 100; r++) {
  const k = r % 7 + 1;
  const m = data.map(x => x * k + 1);
  const f = m.filter(x => x % (k + 2) === 0);
  f.sort((a, b) => b - a);
  const s = f.reduce((c, x) => (c + x) % 1000003, 0);
  tot = (tot + s + f.length) % 1000000007;
}
console.log(`closures ${Math.round(Number(process.hrtime.bigint() - t) / 1e6)} ms ${tot}`);
