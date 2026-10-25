<?php
function work($n) {
    $sum = 0; $s = "";
    for ($i = 0; $i < $n; $i++) {
        $sum += $i % 7;
        if ($i % 100000 == 0) $s .= "x";
    }
    $j = 0;
    while ($j < 1000) { $j++; for ($k = 0; $k < 300; $k++) { $sum ^= $k; } }
    do { $j--; } while ($j > 0);
    return [$sum, $s, $j];
}
class C { public $v = 0; function run() { for ($i = 0; $i < 2000; $i++) { $this->v += $i; } return $this->v; } }
function withStatic() { static $c = 0; for ($i = 0; $i < 1000; $i++) $c++; return $c; }
function byRef(&$a) { for ($i = 0; $i < 1000; $i++) $a[] = $i; return count($a); }
function inTry() { $t = 0; try { for ($i = 0; $i < 1000; $i++) $t += $i; } finally { $t++; } return $t; }
var_dump(work(5000000));
var_dump((new C)->run(), withStatic(), withStatic());
$arr = []; var_dump(byRef($arr), count($arr), inTry());
