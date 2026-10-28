<?php
namespace Spec {
    function sqrt($x) { return 'shadowed'; }
    function shadow($n) { $t = 0.0; for ($i = 0; $i < $n; $i++) { $a = 4.0; $t = $a * 2; $u = sqrt($a); } return [$t, $u ?? null]; }
}
namespace {
class P { public $x = 1.0; public $y = 2.0; public float $tf = 0.5; public $s = 'str'; }
class M { public $x = 1.0; function __get($n) { echo "get($n) "; return 3.0; } }
class Q { public $x = 1.0; }
function mix(array $objs, int $n) {
    $acc = 0.0;
    for ($i = 0; $i < $n; $i++) {
        $o = $objs[$i % count($objs)];
        $d = $o->x - $o->y;
        $acc += $d * 0.5;
        $o->x += $d;
    }
    return $acc;
}
function divs($o, $n) { $r = 0.0; for ($i = 0; $i < $n; $i++) { $q = $o->x / ($o->y - 2.0); $r += $q; } return $r; }
function late($o, $n) { $t = 0.0; for ($i = 0; $i < $n; $i++) { for ($j = 0; $j < $i; $j++) { $w = $o->x * 2; } $t += 1.5; } return [$t, $w ?? 'unset']; }
function grow($o, $n) { $s = 0.0; for ($i = 0; $i < $n; $i++) { $s += $o->x; if (false) {} } return $s; }
function typed($o, $n) { for ($i = 0; $i < $n; $i++) { $v = $o->tf * 2; $o->tf = $v - 0.25; } return $o->tf; }
function before($o, $n) { $k = 1.0; for ($i = 0; $i < $n; $i++) { $o->x = $k; $k = $o->y + $i; } return [$o->x, $k]; }
function thrower($o, $n) { $z = 0.0; for ($i = 0; $i < $n; $i++) { $z += $o->x; $o->x = $z / 2; } return $z; }
for ($r = 0; $r < 3; $r++) {
    $a = new P; $b = new P; $b->y = 'abc';
    try { echo json_encode(mix([$a, new P, $b], 9)), "\n"; } catch (TypeError $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    $p = new P; $p->y = '5';
    echo json_encode(mix([$p, new P], 6)), "\n";
    $m = new M; unset($m->x);
    echo json_encode(mix([new P, $m], 4)), "\n";
    echo json_encode(mix([new Q, new P], 4)), "\n";
    try { echo divs(new P, 3), "\n"; } catch (DivisionByZeroError $e) { echo 'div0: ', $e->getMessage(), "\n"; }
    echo json_encode(late(new P, 4)), json_encode(late(new P, 0)), "\n";
    echo json_encode(Spec\shadow(3)), "\n";
    echo typed(new P, 5), ' ', json_encode(before(new P, 4)), "\n";
    $big = new P; $big->x = 1e308; echo json_encode(grow($big, 3)), ' ', json_encode(is_nan(sqrt(-1.0))), "\n";
    $t = new P; $t->x = 'oops';
    try { echo thrower($t, 3), "\n"; } catch (TypeError $e) { echo 'TypeError: ', $e->getMessage(), "\n"; }
}
}
