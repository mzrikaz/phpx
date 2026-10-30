<?php
// Integer-only arrays sort directly; ties keep their order.
function show($a) { echo json_encode($a), "\n"; }
$a = ['x' => 3, 'y' => 1, 'z' => 3, 'w' => -5, 'v' => 1, 'u' => PHP_INT_MAX, 't' => PHP_INT_MIN];
$b = $a; arsort($b); show($b);
$b = $a; asort($b); show($b);
$b = $a; sort($b); show($b);
$b = $a; rsort($b); show($b);
$b = $a; asort($b, SORT_NUMERIC); show($b);
$b = $a; asort($b, SORT_STRING); show($b);
$b = [3, 1, 2.5, 1]; sort($b); show($b);
$b = [10, 9, "10", 100]; sort($b); show($b);
$c = []; for ($i = 0; $i < 200; $i++) $c["k$i"] = ($i * 37) % 11; arsort($c); show(array_slice($c, 0, 20, true));
