<?php
namespace Calc {
    function A(int $i, int $j): float { return 1.0 / ((($i + $j) * ($i + $j + 1) >> 1) + $i + 1); }
    function scale(float $x, int $k) { return $x * $k + ($k << 2); }
    function big(int $i) { return ($i * 4611686018427387904) * 1.5; }
    function half(float $x): float { return $x / 2; }
    function sq(int $n) { return $n * $n; }
    function inv(float $x): float { return 1 / $x; }
    function run(int $n) {
        $s = 0.0; $t = 0.0; $u = 0.0; $w = 0.0; $q = 0.0;
        for ($i = 0; $i < $n; $i++) {
            $j = $i % 3;
            $s += A($i, $j);
            $t += scale($s, $j);
            $u += big($j) * 0.5;
            $w += half($i) + sq($j) * 1.0;
        }
        return [round($s, 9), round($t, 6), $u, $w];
    }
    function divz(array $xs) { $r = 0.0; for ($i = 0; $i < count($xs); $i++) { $x = $xs[$i]; $r += inv($x); } return $r; }
    function coerce($n) { $s = 0.0; for ($i = 0; $i < $n; $i++) { $f = $i + 0.5; $s += A($f, 1) * 1.0; } return $s; }
    function strs() { $s = 0.0; $v = '3'; for ($i = 0; $i < 3; $i++) { $s += A($v, 1) + 0.5; } return round($s, 9); }
}
namespace {
    if (!function_exists('later')) { function later(int $x) { return $x * 2; } }
    function uses_later($n) { $s = 0.0; for ($i = 0; $i < $n; $i++) { $s += later($i) + 0.5; } return $s; }
    for ($r = 0; $r < 3; $r++) {
        echo json_encode(Calc\run(10)), "\n";
        try { echo Calc\divz([2.0, 4.0, 0.0]), "\n"; } catch (DivisionByZeroError $e) { echo 'div0: ', $e->getMessage(), "\n"; }
        try { echo Calc\coerce(3), "\n"; } catch (TypeError $e) { echo 'TypeError: ', explode(', called', $e->getMessage())[0], "\n"; }
        echo json_encode(Calc\strs()), ' ', uses_later(4), "\n";
    }
}
