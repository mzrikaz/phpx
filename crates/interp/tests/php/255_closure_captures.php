<?php
// Captured values bound inline into compiled closures: every value type,
// writes to a captured copy (never seen by the next call or the outer
// scope), by-reference captures, bound and static closures.
class Box { public $n = 0; public function adder(int $k): Closure { return function (int $x) use ($k) { return $x + $k + $this->n; }; } }
$s = "str"; $a = [1, 2]; $o = new Box; $i = 7; $f = 1.5; $nul = null; $b = true;
$all = fn($x) => [$x, $s, $a, $o->n, $i, $f, $nul, $b];
$grow = function ($x) use ($a, $s) { $a[] = $x; $s .= $x; return [count($a), $s]; };
$count = 0;
$byref = function ($x) use (&$count, $i) { $count += $x + $i; return $count; };
$static = static fn($x) => $x * $i;
$adder = $o->adder(10);
$res = [];
for ($r = 0; $r < 1500; $r++) {
    $o->n = $r % 3;
    $x = $all($r);
    $g = $grow($r);
    $c = $byref(1);
    $st = $static($r);
    $ad = $adder($r);
    $m = array_map(fn($v) => $v + $i + strlen($s), [$r, $r + 1]);
    if ($r % 500 === 1) $res[] = [$x, $g, $c, $st, $ad, $m];
}
echo json_encode($res), "\n";
echo json_encode([$a, $s, $count, $o->n]), "\n";
// A closure made in a loop captures that iteration's values.
$fs = [];
foreach (['a', 'b', 'c'] as $k => $v) { $fs[] = fn() => "$k:$v"; }
$out = [];
for ($r = 0; $r < 1200; $r++) { foreach ($fs as $fn) { $last = $fn(); } $out[$last] = true; }
echo implode(",", array_keys($out)), " ", implode(",", array_map(fn($fn) => $fn(), $fs)), "\n";
// Closure::bind / bindTo keep the captured values.
$base = function () use ($i) { return $i + $this->n; };
$o->n = 100;
$bound = Closure::bind($base, $o, Box::class);
$t = 0; for ($r = 0; $r < 1200; $r++) $t += $bound();
echo $t, "\n";
