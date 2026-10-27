<?php
// spectral-norm: function calls and float math.
function A(int $i, int $j): float { return 1.0 / ((($i + $j) * ($i + $j + 1) >> 1) + $i + 1); }
function Av(int $n, array $v): array { $r = []; for ($i = 0; $i < $n; $i++) { $s = 0.0; for ($j = 0; $j < $n; $j++) $s += A($i, $j) * $v[$j]; $r[$i] = $s; } return $r; }
function Atv(int $n, array $v): array { $r = []; for ($i = 0; $i < $n; $i++) { $s = 0.0; for ($j = 0; $j < $n; $j++) $s += A($j, $i) * $v[$j]; $r[$i] = $s; } return $r; }
function AtAv(int $n, array $v): array { return Atv($n, Av($n, $v)); }
$n = 400; $t = hrtime(true);
$u = array_fill(0, $n, 1.0);
for ($i = 0; $i < 10; $i++) { $v = AtAv($n, $u); $u = AtAv($n, $v); }
$vBv = 0.0; $vv = 0.0; for ($i = 0; $i < $n; $i++) { $vBv += $u[$i] * $v[$i]; $vv += $v[$i] * $v[$i]; }
printf("spectral %.0f ms %.9f\n", (hrtime(true) - $t) / 1e6, sqrt($vBv / $vv));
