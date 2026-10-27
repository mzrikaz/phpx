<?php
class P { public $n = 1; public $f = 1.5; public $s = 'a'; public $arr = [1]; public int $ti = 3; public float $tf = 2.0; public readonly int $ro; public $r;
    function __construct() { $this->ro = 1; } }
class M { private $data = ['v' => 10];
    function __get($k) { echo "get($k) "; return $this->data[$k] ?? null; }
    function __set($k, $v) { echo "set($k) "; $this->data[$k] = $v; } }
function ops($o, $x) {
    $o->n += $x; $o->f *= $x; $o->s .= $x; $o->n -= 1; $o->f /= 2; $o->n %= 7; $o->n **= 2; $o->n |= 8;
    $o->tf += $x; $o->ti += 1;
    return [$o->n, $o->f, $o->s, $o->tf, $o->ti];
}
function arith($a, $b) {
    $r = [];
    foreach ([[$a, $b], [$b, $a], [$a, 0.5], [0.5, $b]] as [$x, $y]) {
        $r[] = $x + $y; $r[] = $x - $y; $r[] = $x * $y;
        try { $r[] = $x / $y; } catch (DivisionByZeroError $e) { $r[] = 'div0'; }
    }
    return $r;
}
for ($i = 0; $i < 3; $i++) {
    $o = new P;
    echo json_encode(ops($o, 2)), json_encode(ops($o, 1.5)), "\n";
    $m = new M; $m->v += 5; echo $m->v, "\n";
    try { $o->ro += 1; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { $o->ti += 1.5; } catch (TypeError $e) { echo get_class($e), "\n"; }
    echo $o->ti, "\n";
    $n = null;
    try { $n->x += 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    $v = 5; $o->r = &$v; $o->r += 2; echo $v, "\n";
    $o->n = PHP_INT_MAX; $o->n += 1; var_dump($o->n);
    echo json_encode(arith(7, 2)), json_encode(arith(7.5, 0)), json_encode(arith(-3, 0.25)), "\n";
    var_dump(10 / 4, 10 / 5, 1.0 / 3, PHP_INT_MAX * 2, -0.0 * 1, 1e308 * 10);
}
