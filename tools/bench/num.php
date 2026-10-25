<?php
function fib($n) { return $n < 2 ? $n : fib($n - 1) + fib($n - 2); }
function loop_sum(int $n): int { $s = 0; for ($i = 0; $i < $n; $i++) { $s += $i * $i % 7; } return $s; }
function is_prime(int $n): bool { if ($n < 2) return false; for ($d = 2; $d * $d <= $n; $d++) { if ($n % $d == 0) return false; } return true; }
function count_primes(int $limit): int { $c = 0; for ($i = 0; $i < $limit; $i++) { if (is_prime($i)) $c++; } return $c; }
function collatz_max(int $limit): int { $best = 0; for ($i = 1; $i < $limit; $i++) { $n = $i; $steps = 0; while ($n != 1) { $n = $n % 2 == 0 ? intdiv($n, 2) : 3 * $n + 1; $steps++; } if ($steps > $best) $best = $steps; } return $best; }
function mandel(float $cx, float $cy): int { $x = 0.0; $y = 0.0; $i = 0; while ($i < 200 && $x * $x + $y * $y < 4.0) { $t = $x * $x - $y * $y + $cx; $y = 2.0 * $x * $y + $cy; $x = $t; $i++; } return $i; }
function mandel_area(int $n): int { $total = 0; for ($a = 0; $a < $n; $a++) { for ($b = 0; $b < $n; $b++) { $total += mandel(-2.0 + 3.0 * $a / $n, -1.5 + 3.0 * $b / $n); } } return $total; }
function t(string $label, callable $f) { $t = microtime(true); $r = $f(); printf("%-14s %12s %7.1f ms\n", $label, $r, (microtime(true) - $t) * 1000); }
t("fib(30)", fn() => fib(30));
t("loop_sum", fn() => loop_sum(5000000));
t("count_primes", fn() => count_primes(300000));
t("collatz", fn() => collatz_max(300000));
t("mandelbrot", fn() => mandel_area(400));
