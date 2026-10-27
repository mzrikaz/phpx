class Body { constructor(x, y, z, vx, vy, vz, m) { this.x = x; this.y = y; this.z = z; this.vx = vx; this.vy = vy; this.vz = vz; this.m = m; } }
function energy(bs) {
  let e = 0.0; const n = bs.length;
  for (let i = 0; i < n; i++) {
    const b = bs[i];
    e += 0.5 * b.m * (b.vx * b.vx + b.vy * b.vy + b.vz * b.vz);
    for (let j = i + 1; j < n; j++) {
      const c = bs[j]; const dx = b.x - c.x, dy = b.y - c.y, dz = b.z - c.z;
      e -= (b.m * c.m) / Math.sqrt(dx * dx + dy * dy + dz * dz);
    }
  }
  return e;
}
function advance(bs, dt) {
  const n = bs.length;
  for (let i = 0; i < n; i++) {
    const b = bs[i];
    for (let j = i + 1; j < n; j++) {
      const c = bs[j]; const dx = b.x - c.x, dy = b.y - c.y, dz = b.z - c.z;
      const d2 = dx * dx + dy * dy + dz * dz; const mag = dt / (d2 * Math.sqrt(d2));
      b.vx -= dx * c.m * mag; b.vy -= dy * c.m * mag; b.vz -= dz * c.m * mag;
      c.vx += dx * b.m * mag; c.vy += dy * b.m * mag; c.vz += dz * b.m * mag;
    }
  }
  for (const b of bs) { b.x += dt * b.vx; b.y += dt * b.vy; b.z += dt * b.vz; }
}
const pi = 3.141592653589793, sm = 4 * pi * pi, dp = 365.24;
const bs = [
  new Body(0, 0, 0, 0, 0, 0, sm),
  new Body(4.84143144246472090e+00, -1.16032004402742839e+00, -1.03622044471123109e-01, 1.66007664274403694e-03 * dp, 7.69901118419740425e-03 * dp, -6.90460016972063023e-05 * dp, 9.54791938424326609e-04 * sm),
  new Body(8.34336671824457987e+00, 4.12479856412430479e+00, -4.03523417114321381e-01, -2.76742510726862411e-03 * dp, 4.99852801234917238e-03 * dp, 2.30417297573763929e-05 * dp, 2.85885980666130812e-04 * sm),
  new Body(1.28943695621391310e+01, -1.51111514016986312e+01, -2.23307578892655734e-01, 2.96460137564761618e-03 * dp, 2.37847173959480950e-03 * dp, -2.96589568540237556e-05 * dp, 4.36624404335156298e-05 * sm),
  new Body(1.53796971148509165e+01, -2.59193146099879641e+01, 1.79258772950371181e-01, 2.68067772490389322e-03 * dp, 1.62824170038242295e-03 * dp, -9.51592254519715870e-05 * dp, 5.15138902046611451e-05 * sm),
];
let px = 0, py = 0, pz = 0;
for (const b of bs) { px += b.vx * b.m; py += b.vy * b.m; pz += b.vz * b.m; }
bs[0].vx = -px / sm; bs[0].vy = -py / sm; bs[0].vz = -pz / sm;
const t = process.hrtime.bigint();
for (let i = 0; i < 1000000; i++) advance(bs, 0.01);
console.log(`nbody ${Math.round(Number(process.hrtime.bigint() - t) / 1e6)} ms ${energy(bs).toFixed(9)}`);
