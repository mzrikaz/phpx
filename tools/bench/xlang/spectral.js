function A(i, j) { return 1.0 / ((((i + j) * (i + j + 1)) >> 1) + i + 1); }
function Av(n, v) { const r = []; for (let i = 0; i < n; i++) { let s = 0.0; for (let j = 0; j < n; j++) s += A(i, j) * v[j]; r[i] = s; } return r; }
function Atv(n, v) { const r = []; for (let i = 0; i < n; i++) { let s = 0.0; for (let j = 0; j < n; j++) s += A(j, i) * v[j]; r[i] = s; } return r; }
function AtAv(n, v) { return Atv(n, Av(n, v)); }
const n = 400, t = process.hrtime.bigint();
let u = new Array(n).fill(1.0), v;
for (let i = 0; i < 10; i++) { v = AtAv(n, u); u = AtAv(n, v); }
let vBv = 0.0, vv = 0.0; for (let i = 0; i < n; i++) { vBv += u[i] * v[i]; vv += v[i] * v[i]; }
console.log(`spectral ${Math.round(Number(process.hrtime.bigint() - t) / 1e6)} ms ${Math.sqrt(vBv / vv).toFixed(9)}`);
