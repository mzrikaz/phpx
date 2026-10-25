<?php
// Hot numeric functions run natively; every case must still match PHP exactly.
function add($a, $b) { return $a + $b; }
function mul(int $a, int $b) { return $a * $b; }
function grow(int $n) { $x = 1; for ($i = 0; $i < $n; $i++) { $x = $x * 3; } return $x; }  // overflows to float
function mod($a, $b) { return $a % $b; }
function shl(int $a, int $b) { return $a << $b; }
function shr(int $a, int $b) { return $a >> $b; }
function fdiv2(float $a, float $b) { return $a / $b; }
function cmp(float $a, float $b) { return [$a < $b, $a == $b, $a != $b, $a >= $b]; }
function toint(float $f) { return (int) $f; }
function neg($x) { return -$x; }
function fact($n) { return $n <= 1 ? 1 : $n * fact($n - 1); }
function depth($n) { return $n == 0 ? 0 : 1 + depth($n - 1); }
function widen(float $x) { return $x * 2; }
function logic($a, $b) { return ($a && $b) || (!$a xor $b); }
function absmin($a, $b) { return abs(min($a, $b)) + max($a, $b); }
function sqr($x) { return sqrt($x) + floor($x / 3.0) + ceil($x / 7.0); }
function spaceship(int $a, int $b) { return $a <=> $b; }
function bits(int $a) { return (~$a & 0xFF) | ($a ^ 3); }
function loopbreak(int $n) { $s = 0; for ($i = 0; ; $i++) { if ($i % 3 == 0) continue; if ($i > $n) break; $s += $i; } return $s; }
function nested(int $n) { $c = 0; $i = 0; while (true) { $i++; for ($j = 0; $j < $i; $j++) { if ($j == 5) continue 2; if ($i > $n) break 2; $c++; } } return $c; }
function dowhile(int $n) { $i = 0; do { $i += 2; } while ($i < $n); return $i; }
function ternary_short($x) { return $x ?: 42; }
for ($k = 0; $k < 60; $k++) {           // make them hot
    add(1, 2); mul(3, 4); mod(7, 3); neg(5); fact(5); widen(1); logic(1, 0); absmin(-3, 2); spaceship(1, 2); bits(5); ternary_short(0);
    add(1.5, 2); shl(1, 2); shr(8, 1); fdiv2(1.0, 2.0); toint(1.5); neg(0.5); sqr(4); sqr(2.5); depth(3); logic(0.0, true);
    absmin(-2.5, -1.0); ternary_short(1.5); grow(2); loopbreak(3); nested(2); dowhile(3); widen(1.5);
}
var_dump(add(PHP_INT_MAX, 1), add(1.5, 2), add(2, 3), mul(PHP_INT_MAX, 2), grow(10), grow(50));
var_dump(mod(-7, 3), mod(PHP_INT_MIN, -1), mod(10, -3));
try { mod(1, 0); } catch (DivisionByZeroError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump(shl(1, 63), shl(1, 64), shl(5, 70), shr(-16, 2), shr(-1, 100), shr(1024, 64));
try { shl(1, -1); } catch (ArithmeticError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump(fdiv2(1.0, 3.0), cmp(1.0, 2.0), cmp(NAN, 1.0), cmp(NAN, NAN));
try { fdiv2(1.0, 0.0); } catch (DivisionByZeroError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump(toint(3.99), toint(-3.99), toint(1e19), toint(NAN), neg(PHP_INT_MIN), neg(-0.0), neg(5));
var_dump(fact(20), fact(21), fact(25));
var_dump(depth(10000));
var_dump(widen(3), widen(1.5), logic(0, 0), logic(1, 1), logic(0.0, true), absmin(-7, 3), absmin(-2.5, -1.0));
var_dump(sqr(16), sqr(2.25), spaceship(PHP_INT_MIN, PHP_INT_MAX), bits(0), bits(-1));
var_dump(loopbreak(20), nested(8), dowhile(9), ternary_short(0), ternary_short(7), ternary_short(0.0));
// Types vary between calls: separate specializations, or interpreter.
function poly($x) { return $x + $x; }
for ($k = 0; $k < 60; $k++) { poly($k); poly($k + 0.5); }
var_dump(poly(2), poly(2.25), poly(PHP_INT_MAX), poly("3"), poly(true));
function maybe($x) { if ($x > 0) { $r = 1; } else { $r = 1.5; } return $r; }  // var changes type: not compiled
var_dump(maybe(1), maybe(-1));
function undef($x) { if ($x) { $y = 2; } return $x + 1; }
var_dump(undef(1), undef(0));
