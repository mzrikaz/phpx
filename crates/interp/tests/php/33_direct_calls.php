<?php
function fact($n) { return $n <= 1 ? 1 : $n * fact($n - 1); }
function even($n) { return $n == 0 ? true : odd($n - 1); }
function odd($n) { return $n == 0 ? false : even($n - 1); }
function thrower($i) { if ($i % 3 == 0) throw new RuntimeException("bad $i"); return $i; }
function counter() { static $c = 0; return ++$c; }
function useGlobal() { global $g; $g++; return $g; }
function swap(&$a, &$b) { [$a, $b] = [$b, $a]; }
function defs($a, $b = 10, $c = "x") { return "$a-$b-$c"; }
function extra($a) { return $a; }
function typed(int $a, ?string $s = null): string { return $a . ($s ?? "-"); }
function nothing() {}
function warnAfter() { $arr = []; return $arr; }
function deep($n) { return $n == 0 ? 0 : 1 + deep($n - 1); }
$g = 0;
$out = [];
for ($i = 0; $i < 6; $i++) {
    $out[] = fact(10) . " " . (even(10) ? "e" : "o") . " " . counter() . " " . useGlobal();
    try { $out[] = thrower($i); } catch (RuntimeException $e) { $out[] = $e->getMessage() . "@" . $e->getLine(); }
    $x = "l$i"; $y = "r$i"; swap($x, $y); $out[] = "$x$y";
    $out[] = defs($i) . " " . defs($i, 2) . " " . defs($i, 2, "z") . " " . extra($i, 99);
    $out[] = typed($i) . typed($i, "s") . typed("$i");
    $out[] = var_export(nothing(), true);
    $r = warnAfter(); $out[] = count($r);
}
$out[] = deep(5000);
echo implode("\n", $out), "\n";
try { typed("abc"); } catch (TypeError $e) { echo str_replace(__FILE__, 'F', $e->getMessage()), "\n"; }
