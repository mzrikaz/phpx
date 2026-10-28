<?php
function dot(array $a, array $b, int $n) { $s = 0.0; for ($i = 0; $i < $n; $i++) { $s += $a[$i] * $b[$i]; } return $s; }
function rows(array $m, int $n) { $out = []; for ($i = 0; $i < $n; $i++) { $t = 0.0; for ($j = 0; $j < $n; $j++) { $t += $m[$j] * 0.5; } $out[$i] = $t; } return $out; }
function lit(array $v) { $s = 0.0; for ($i = 0; $i < 3; $i++) { $s += $v[0] + $v[2]; } return $s; }
function long_run($n) { $acc = 0.0; $v = [1.5, 2.5, 3.5]; for ($i = 0; $i < $n; $i++) { $k = $i % 3; $acc += $v[$k] * 0.001; } return round($acc, 6); }
$f = [1.5, 2.0, 0.5]; $g = [2.0, 4.0, 1.0];
for ($r = 0; $r < 3; $r++) {
    echo dot($f, $g, 3), ' ', dot([1, 2, 3], [0.5, 0.5, 0.5], 3), ' ', dot(['a' => 1.0], [1.0], 1), "\n";
    $h = [0 => 1.5, 2 => 2.5, 1 => 3.5];
    echo dot($h, $h, 3), ' ', json_encode(rows([0.5, 1.0, 1.5, 2.0], 4)), "\n";
    $x = 2.0; $refs = [1.0, &$x, 3.0];
    echo dot($refs, $refs, 3), ' ', lit([1.0, 2.0, 4.0]), "\n";
    try { echo dot([1.5], [2.5], 2), "\n"; } catch (Throwable $e) { echo get_class($e), "\n"; }
    echo json_encode(dot(['1.5', 2.0], [2.0, 2.0], 2)), ' ', json_encode(dot([null, 1.0], [1.0, 1.0], 2)), "\n";
    echo long_run(30000), "\n";
}
