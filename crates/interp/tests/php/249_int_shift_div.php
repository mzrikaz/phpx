<?php
function ops($x, $y) {
    $r = [];
    foreach ([[$x, $y], [$y, $x]] as [$a, $b]) {
        try { $r[] = $a << $b; } catch (ArithmeticError $e) { $r[] = 'shl:' . $e->getMessage(); }
        try { $r[] = $a >> $b; } catch (ArithmeticError $e) { $r[] = 'shr:' . $e->getMessage(); }
        try { $r[] = $a / $b; } catch (DivisionByZeroError $e) { $r[] = 'div:' . $e->getMessage(); }
    }
    return $r;
}
$cases = [[7, 2], [-7, 2], [6, 3], [1, 63], [1, 64], [-1, 70], [5, -1], [PHP_INT_MIN, -1], [PHP_INT_MAX, 1], [9, 0], [0, 5], [10, 4], [-10, 4], [2.5, 2], [true, 1], ['8', '2']];
for ($i = 0; $i < 3; $i++) {
    foreach ($cases as [$x, $y]) { echo json_encode(ops($x, $y)), "\n"; }
    $s = 0; for ($k = 1; $k < 50; $k++) { $s += (($k * 37) >> 2) + (1 << ($k % 60)) % 97 + intdiv($k, 3) + $k / 7; } echo $s, "\n";
}
