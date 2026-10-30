<?php
// Float property updates inside speculated loops (float locals in registers).
class P { public float $x = 1.0; public $u = 0; public int $i = 3; public readonly float $ro; private float $priv = 2.0;
  function __construct() { $this->ro = 1.0; } function priv() { return $this->priv; } }
function loop(P $p, P $q, int $n) {
    $out = [];
    for ($k = 0; $k < $n; $k++) {
        $d = $k * 0.5 + 1.0;
        $p->x += $d * 2.0;
        $p->u += $d;
        $q->x -= $d / 4.0;
        $p->x *= 1.0;
        $q->u = $d * 3.0;
    }
    $out[] = [$p->x, $p->u, $q->x, $q->u];
    for ($k = 0; $k < 3; $k++) {
        $d = $k + 0.5;
        try { $p->i += $d; } catch (TypeError $e) { $out[] = $e->getMessage(); }
        try { $p->ro += $d; } catch (Error $e) { $out[] = $e->getMessage(); }
        try { $p->priv += $d; } catch (Error $e) { $out[] = $e->getMessage(); }
        try { $z = $d - $d; $p->u /= $z; } catch (DivisionByZeroError $e) { $out[] = $e->getMessage(); }
    }
    $out[] = [$p->x, $p->i, $p->ro, $p->priv()];
    return $out;
}
for ($r = 0; $r < 3; $r++) { $p = new P; echo json_encode(loop($p, $p, 50)), "\n"; echo json_encode(loop(new P, new P, 7)), "\n"; }
