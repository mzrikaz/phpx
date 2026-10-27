<?php
// Functions called once with long arrays: compiled at the foreach.
function sum($a) { $s = 0; foreach ($a as $v) { $s += $v; } return $s; }
function keys_after($a) { $n = 0; foreach ($a as $k => $v) { $n++; } return "$n $k $v"; }
function early($a) { foreach ($a as $k => $v) { if ($v === 777) return "found at $k"; } return 'none'; }
function brk($a) { $c = 0; foreach ($a as $v) { if ($v % 2) continue; if ($v > 900) break; $c++; } return "$c $v"; }
function modify($a) { $out = 0; foreach ($a as $k => $v) { $a[$k + 5000] = 1; unset($a[0]); $out++; } return $out . ' ' . count($a); }
function nested($a) { $t = 0; for ($i = 0; $i < 3; $i++) { foreach ($a as $v) { $t += $v * $i; } } return $t; }
function before_and_after($a) { $pre = 'pre'; $x = 5; foreach ($a as $v) { $x += $v; } $post = $pre . $x; return $post; }
function strs($a) { $s = ''; foreach ($a as $k => $v) { if ($k < 3) $s .= $v; } return $s; }
function by_ref_outer($a) { $total = 0; $f = function () use (&$total) { return $total; }; foreach ($a as $v) { $total += $v; } return $f(); }
function gen($a) { foreach ($a as $v) { if ($v % 250 == 0) yield $v; } }
class C { public $n = 0; function run($a) { foreach ($a as $v) { $this->n += $v; } return $this->n; } }
$a = range(0, 999);
echo sum($a), "\n", keys_after($a), "\n", early($a), "\n", brk($a), "\n", modify($a), "\n", nested($a), "\n", before_and_after($a), "\n";
echo strs(array_map(fn($i) => chr(97 + $i % 26), $a)), "\n";
echo by_ref_outer($a), "\n", implode(',', iterator_to_array(gen($a), false)), "\n", (new C)->run($a), "\n";
$assoc = []; foreach ($a as $v) { $assoc["k$v"] = $v; }
echo keys_after($assoc), "\n", sum([]), "\n";
