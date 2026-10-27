<?php
// Method dispatch through interfaces and inheritance, getters/setters, small objects.
interface Shape { public function area(): float; public function scale(float $f): static; }
abstract class Base implements Shape { protected string $name; public function __construct(string $n) { $this->name = $n; } public function name(): string { return $this->name; } }
final class Rect extends Base { public function __construct(private float $w, private float $h) { parent::__construct('rect'); } public function area(): float { return $this->w * $this->h; } public function scale(float $f): static { return new Rect($this->w * $f, $this->h * $f); } }
final class Circle extends Base { public function __construct(private float $r) { parent::__construct('circle'); } public function area(): float { return 3.14159 * $this->r * $this->r; } public function scale(float $f): static { return new Circle($this->r * $f); } }
final class Acc { private float $sum = 0.0; private int $n = 0; public function add(Shape $s): void { $this->sum += $s->area(); $this->n++; } public function total(): float { return $this->sum; } public function count(): int { return $this->n; } }
$t = hrtime(true);
$shapes = []; for ($i = 0; $i < 1000; $i++) { $shapes[] = $i % 2 ? new Rect($i % 10 + 1, $i % 7 + 1) : new Circle($i % 5 + 1); }
$acc = new Acc; $names = 0;
for ($r = 0; $r < 300; $r++) {
    foreach ($shapes as $s) { $acc->add($s->scale(1.01)); if ($s->name() === 'rect') $names++; }
}
printf("oop %.0f ms %.3f %d %d\n", (hrtime(true) - $t) / 1e6, $acc->total(), $acc->count(), $names);
