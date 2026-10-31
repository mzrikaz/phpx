<?php
class A { public $x = null; public $y = 0; private $p = "s"; public $arr = [];
  function __get($n) { echo "get $n\n"; return $n === "magic" ? 1 : null; }
  function t() { return [$this->p === null, !$this->p, $this->x ? 1 : 2, $this->magic === null, $this->none ?? 'd']; } }
class B extends A { public $r; }
$a = new A; $b = new B; $r = 0;
for ($i = 0; $i < 3000; $i++) {
  $o = $i % 2 ? $a : $b;
  if ($o->x === null) $r++;
  if (!$o->y) $r += 2;
  while ($o->y) { $o->y--; }
  $r += $o->x !== null ? 10 : 20;
  if ($i == 1500) { $a->x = "v"; $q = &$b->x; $q = 5; }
  if ($o->arr && $o->y || !$o->x) $r++;
  unset($o->dyn);
}
echo $r, "\n"; print_r($a->t()); unset($a->x); var_dump($a->x === null); var_dump($b->x === null);
class WO { public $p = 1; public $a = [1]; static $s = 0; }
function f(&$x) { $x++; }
function g($x) { return $x + 1; }
class WM { function m(&$x) { $x .= "a"; } function v($x) { return $x; } static function s(&$x) { $x = $x * 2; } }
$o2 = new WO; $m = new WM; $t = 0;
for ($i = 0; $i < 3000; $i++) {
  f($o2->p); $t += g($o2->p);
  $o2->a[0] = "";
  $m->m($o2->a[0]); $t += strlen($m->v($o2->a[0]));
  WO::$s = 1; WM::s(WO::$s); $t += WO::$s;
  $n = new ArrayObject($o2->a);
}
echo $o2->p, " ", $t, " ", $o2->a[0], "\n";
