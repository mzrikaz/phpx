<?php
// Constructors that only store promoted parameters, from compiled `new` sites.
class P { public function __construct(public int $i, public float $f = 1.5, private ?P $next = null, protected string $s = "d", public readonly bool $b = false, public $any = []) {}
  function next() { return $this->next; } function s() { return $this->s; } }
class Q extends P {}
class U { public function __construct(public int|string $v) {} }
class Empty0 { public function __construct() {} }
function mk($k) {
    $out = [];
    $a = new P(1); $out[] = [$a->i, $a->f, $a->next(), $a->s(), $a->b, $a->any];
    $b = new P(2, 3, $a, "x", true, null); $out[] = [$b->i, $b->f, $b->next() === $a, $b->s(), $b->b, $b->any];
    $c = new Q(3, 2.5, new Q(4)); $out[] = [get_class($c), get_class($c->next()), $c->next()->i];
    try { new P("7"); $out[] = "coerced"; } catch (TypeError $e) { $out[] = $e->getMessage(); }
    $d = new P("8"); $out[] = [$d->i];
    try { new P("x"); } catch (TypeError $e) { $out[] = substr($e->getMessage(), 0, 60); }
    try { new P(1, 2.0, new stdClass); } catch (TypeError $e) { $out[] = substr($e->getMessage(), 0, 60); }
    try { new P(); } catch (ArgumentCountError $e) { $out[] = substr($e->getMessage(), 0, 50); }
    $e = new P(1, 2, null, "s", false, [1], "extra"); $out[] = [$e->i, $e->any];
    try { $e->b = true; } catch (Error $x) { $out[] = $x->getMessage(); }
    $u = new U("s"); $out[] = $u->v; $u2 = new U(5); $out[] = $u2->v;
    $z = new Empty0; $out[] = get_class($z);
    $out[] = $k;
    return $out;
}
for ($k = 0; $k < 3; $k++) echo json_encode(mk($k)), "\n";
