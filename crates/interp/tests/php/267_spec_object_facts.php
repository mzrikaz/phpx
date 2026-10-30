<?php
// Speculated loops reading and writing object properties while objects change shape or variables change.
#[AllowDynamicProperties]
class V { public $x = 1.0; public $y = 2.0; function grow($k) { $this->{"p$k"} = $k; return 0.5; } function shrink($k) { unset($this->{"p$k"}); } }
function f(array $vs, int $n) {
    $a = $vs[0]; $b = $vs[1]; $t = 0.0;
    for ($i = 0; $i < $n; $i++) {
        $d = $a->x * 0.5 + $b->y;
        $a->x += $d * 0.001;
        $e = $a->grow($i) + $a->x;           // a call: adds properties (reallocates slots)
        $a->y -= $e * 0.001;
        $t += $a->x + $a->y;
        $b = $vs[$i > 3 ? 2 : 1];            // the variable changes
        $b->x *= 1.0001;
        $f = $b->x + $a->y;
        $b->y = $f;
        $a->shrink($i - 1);                  // unset: a different shape
        $t += $b->y * 0.0;
    }
    return [$t, $a->x, $a->y, $b->x, $b->y, count(get_object_vars($a))];
}
for ($r = 0; $r < 3; $r++) { echo json_encode(f([new V, new V, new V], 40)), "\n"; }
function g(array $vs) {
    $t = 0.0;
    for ($i = 0; $i < 30; $i++) { $o = $vs[$i % 3]; $t += $o->x * 2.0; $o->x += 1.0; }
    return [$t, $vs[0]->x, is_object($vs[2]) ? $vs[2]->x : $vs[2]];
}
for ($r = 0; $r < 3; $r++) { echo json_encode(g([new V, new V, new V])), "\n"; }
try { echo json_encode(g([new V, new V, 5])), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
