class Base { constructor(n) { this._name = n; } name() { return this._name; } }
class Rect extends Base { constructor(w, h) { super('rect'); this.w = w; this.h = h; } area() { return this.w * this.h; } scale(f) { return new Rect(this.w * f, this.h * f); } }
class Circle extends Base { constructor(r) { super('circle'); this.r = r; } area() { return 3.14159 * this.r * this.r; } scale(f) { return new Circle(this.r * f); } }
class Acc { constructor() { this.sum = 0.0; this.n = 0; } add(s) { this.sum += s.area(); this.n++; } total() { return this.sum; } count() { return this.n; } }
const t = process.hrtime.bigint();
const shapes = []; for (let i = 0; i < 1000; i++) shapes.push(i % 2 ? new Rect(i % 10 + 1, i % 7 + 1) : new Circle(i % 5 + 1));
const acc = new Acc(); let names = 0;
for (let r = 0; r < 300; r++) { for (const s of shapes) { acc.add(s.scale(1.01)); if (s.name() === 'rect') names++; } }
console.log(`oop ${Math.round(Number(process.hrtime.bigint() - t) / 1e6)} ms ${acc.total().toFixed(3)} ${acc.count()} ${names}`);
