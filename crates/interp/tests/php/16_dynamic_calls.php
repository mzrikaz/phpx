<?php
class P { public $x = 1; public $y = 2;
  function add($a, $b = 10, ...$r) { return $a + $b + array_sum($r); }
  static function mk(int $x, int $y = 0) { return $x * 100 + $y; }
  function __construct($x = 1, $y = 2) { $this->x = $x; $this->y = $y; }
}
function f($a, $b = 5, $c = 7) { return "$a-$b-$c"; }
function run() {
  $f = 'f'; echo $f(1), "\n";
  $g = fn($x) => $x * 2; echo $g(21), "\n";
  $h = [new P, 'add']; echo $h(1, 2), "\n";
  echo f(...[1, 2, 3]), "\n";
  echo f(1, c: 9), "\n";
  echo f(...['c' => 3, 'a' => 4]), "\n";
  $p = new P(y: 9, x: 3); echo $p->x, $p->y, "\n";
  $n = 'y'; echo $p->$n, "\n";
  $m = 'add'; echo $p->$m(1, 2, 3, 4), "\n";
  echo $p->add(...[1, 2, 3]), "\n";
  echo P::mk(y: 4, x: 2), "\n";
  $s = 'strtoupper'; echo $s('abc'), "\n";
  $cl = function($a) { return $a . '!'; }; echo $cl(...['hi']), "\n";

}
for ($i = 0; $i < 3; $i++) run();
