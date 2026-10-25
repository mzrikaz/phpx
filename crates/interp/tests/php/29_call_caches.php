<?php
// Numeric JIT entries cached per call site, and their fallbacks.
function add($a, $b) { return $a + $b; }
function half($x) { return $x / 2; }
function big($x) { return $x * 4611686018427387904; }
function nothing() {}
function run() {
    $s = 0;
    for ($i = 0; $i < 300; $i++) { $s = add($s, $i); }
    echo $s, "\n";
    $f = 0.0;
    for ($i = 0; $i < 300; $i++) { $f = add($f, 0.5); }
    echo $f, "\n";
    // Types change at the same site.
    $r = [];
    foreach ([1, 2.5, true, 4, "5", null, 7] as $v) { $r[] = var_export(half($v), true); }
    echo implode(" ", $r), "\n";
    // Overflow deoptimizes to floats.
    $r = [];
    for ($i = 0; $i < 100; $i++) { $r[] = big($i % 3); }
    echo var_export(array_slice($r, 0, 6), true), "\n";
    for ($i = 0; $i < 100; $i++) { $n = nothing(); }
    var_dump($n);
}
run();
