<?php
class A { public $x = 1; protected $p = 2; private $q = 3; public int $t = 0; public readonly int $r;
  function __construct() { $this->r = 9; }
  function bump() { for ($i = 0; $i < 3; $i++) { $this->p = $this->p + 1; $this->q = $this->q * 2; } return [$this->p, $this->q]; }
  function getQ() { return $this->q; } }
class B extends A { private $z = "b"; function getQ2() { return $this->z; } }
function rd($o) { return $o->x; }
function wr($o, $v) { $o->x = $v; return $o; }
$a = new A; $b = new B; $s = new stdClass;
for ($i = 0; $i < 4; $i++) {
  echo rd($a), rd($b), " "; wr($a, $i); wr($b, "s$i"); echo $a->x, $b->x, " ";
  echo json_encode($a->bump()), $b->getQ(), $b->getQ2(), " ";
  $s->{"k$i"} = $i; $s->x = $i; echo rd($s), " ";
}
echo "\n";
try { $o = null; $o->x = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { wr($a, 1); $a->t = "abc"; } catch (TypeError $e) { echo "TE ", $e->getMessage(), "\n"; }
try { $a->r = 5; } catch (Error $e) { echo $e->getMessage(), "\n"; }
$x = 5; $c = new A; $c->x = &$x; $x = 7; echo rd($c), rd($c), "\n"; wr($c, 8); echo $x, "\n";
unset($c->x); echo isset($c->x) ? "set" : "unset", " "; $c->x = 3; echo rd($c), json_encode(get_object_vars($c)), "\n";
$d = new stdClass; $d->a = 1; $d->b = 2; unset($d->a); $d->a = 3; echo json_encode($d), "\n";
for ($i = 0; $i < 3; $i++) { $m = new class { public $v = 1; }; $m->v += $i; echo rd2($m); } function rd2($o) { return $o->v; }
echo "\n";
class M { public function __get($n) { return "magic $n"; } } echo (new M)->zz, " ", rd(new M), "\n";
