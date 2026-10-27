const t = process.hrtime.bigint(); let tot = 0;
const doc = []; for (let i = 0; i < 500; i++) doc.push({ id: i, name: `user${i}`, active: i % 2 == 0, score: i * 1.5, tags: ['x', 'y', `t${i}`], meta: { a: i, b: null } });
for (let r = 0; r < 100; r++) {
  const s = JSON.stringify(doc).replace(/(\d)\.0([,}])/g, '$1$2');
  const back = JSON.parse(s);
  tot = (tot + s.length + back.length + back[r].id) % 1000000007;
}
console.log(`json ${Math.round(Number(process.hrtime.bigint() - t) / 1e6)} ms ${tot}`);
