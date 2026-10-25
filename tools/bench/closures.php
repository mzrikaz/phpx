<?php
function b1($N) { $f = fn($x) => $x + 1; $s = 0; for ($i = 0; $i < $N; $i++) { $s = $f($s); } return $s; }
function b2($N) { $a = range(1, 100); $s = 0; for ($i = 0; $i < $N / 100; $i++) { $s += count(array_map(fn($x) => $x * 2, $a)); } return $s; }
function b3($N) { $s = 0; for ($i = 0; $i < $N; $i++) { $f = function () use ($i) { return $i; }; $s += $f(); } return $s; }
$N = 2000000;
foreach (['b1'=>'call closure','b2'=>'array_map cb','b3'=>'make+call'] as $f => $d) { $t = microtime(true); $f($N); printf("%-14s %5.1f ns\n", $d, (microtime(true)-$t)*1e9/$N); }
