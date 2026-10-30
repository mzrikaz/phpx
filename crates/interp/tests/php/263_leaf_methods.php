<?php
// Small pure methods called without a frame; anything unusual takes the real call.
interface Shape { function area(): float; }
class Rect implements Shape { function __construct(private float $w, private float $h) {} function area(): float { return $this->w * $this->h; } }
class Circle implements Shape { function __construct(private float $r) {} function area(): float { return 3.14159 * $this->r * $this->r; } }
class Sq extends Rect { public $extra = 1; }
class Tri implements Shape { public float $b = 2.0; public float $h; function area(): float { return $this->b * $this->h / 2; } }
class N {
    public int $n = 5; public float $f = 1.5; public bool $on = true; public int $big = PHP_INT_MAX;
    function inc(int $by): int { return $this->n + $by; }
    function over(): int { return $this->big + 1; }
    function overU() { return $this->big * 2; }
    function div(float $d): float { return $this->f / $d; }
    function mod(int $m): int { return $this->n % $m; }
    function pos(): bool { return $this->n > 0 && $this->on; }
    function cmp(float $x): bool { return $this->n < $x; }
    function same(int $x): bool { return $this->n === $x; }
    function eqf(float $x): bool { return $this->n == $x; }
    function pick(bool $c): int { return $c ? $this->n : -$this->n; }
    function neg(): int { return -$this->n; }
    function not(): bool { return !$this->on; }
    function mix(int $a, float $b, bool $c): float { return $c ? $a * $b : $a - $b; }
    function untyped() { return $this->n * 2; }
}
function shapes(array $xs) { $t = 0.0; foreach ($xs as $x) { $t += $x->area(); } return $t; }
function probe(N $o) {
    $r = [$o->inc(3), $o->div(4), $o->mod(3), $o->pos(), $o->cmp(5.5), $o->cmp(4), $o->same(5), $o->same(6), $o->eqf(5.0),
          $o->pick(true), $o->pick(false), $o->neg(), $o->not(), $o->mix(2, 1.5, true), $o->mix(2, 1.5, false), $o->untyped(), $o->overU()];
    try { $r[] = $o->over(); } catch (Throwable $e) { $r[] = get_class($e) . ": " . $e->getMessage(); }
    try { $r[] = $o->div(0); } catch (DivisionByZeroError $e) { $r[] = $e->getMessage() . " @" . $e->getLine() . " " . $e->getTrace()[0]['function']; }
    try { $r[] = $o->mod(0); } catch (DivisionByZeroError $e) { $r[] = $e->getMessage(); }
    $r[] = $o->mod(-1);
    try { $r[] = $o->inc("x"); } catch (TypeError $e) { $r[] = strstr($e->getMessage(), ", called", true); }
    $r[] = $o->inc("7"); $r[] = $o->inc(true); $r[] = $o->div(2); $r[] = $o->pick(0); $r[] = $o->mix(1, 2, 3);
    return $r;
}
for ($k = 0; $k < 3; $k++) {
    echo shapes([new Rect(2, 3), new Circle(1), new Sq(1, 2), new Rect(1.5, 2)]), "\n";
    $s = new Rect(1, 1); $s->dyn = 5; echo shapes([$s, $s]), "\n";
    $t = new Tri; try { echo shapes([$t]), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    $t->h = 3; echo shapes([$t, $t]), "\n";
    $o = new N; echo json_encode(probe($o)), "\n";
    $o->n = -2; $o->on = false; echo json_encode(probe($o)), "\n";
    $o->n = PHP_INT_MIN; echo json_encode([$o->inc(1), $o->untyped()]), "\n";
    try { $o->neg(); } catch (TypeError $e) { echo $e->getMessage(), " ", $e->getTrace()[0]['function'], "\n"; }
}
