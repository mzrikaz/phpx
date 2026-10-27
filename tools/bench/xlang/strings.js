const t = process.hrtime.bigint(); let h = 0;
for (let r = 0; r < 200; r++) {
  const parts = [];
  for (let i = 0; i < 1000; i++) parts.push('item-' + i + ':' + (i * 7 % 13));
  let s = parts.join(',');
  s = s.split('item').join('Elem');
  const up = s.toUpperCase();
  let cnt = 0; for (const p of up.split(',')) { if (p.includes(':1')) cnt++; cnt += p.substr(2, 3).length; }
  let buf = ''; for (let i = 0; i < 2000; i++) buf += `x${i}y`;
  h = (h + cnt + buf.length + up.indexOf('ELEM-999')) % 1000000007;
}
console.log(`strings ${Math.round(Number(process.hrtime.bigint() - t) / 1e6)} ms ${h}`);
