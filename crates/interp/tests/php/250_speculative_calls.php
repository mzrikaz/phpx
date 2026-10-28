<?php
$calls = 0;
function val($i) { global $calls; $calls++; return match ($i % 5) { 0 => 1.5, 1 => 2, 2 => 0.25, 3 => '3', default => 1.0 }; }
function loud($i) { echo "[$i]"; return $i * 0.5; }
function weird($i) { return $i == 3 ? [1] : 1.0; }
function boom($i) { if ($i == 2) throw new RuntimeException("boom at $i"); return 1.0; }
function sum_val($n) { $s = 0.0; for ($i = 0; $i < $n; $i++) { $s += val($i) * 2.0; } return $s; }
function sum_loud($n, array $w) { $s = 0.0; for ($i = 0; $i < $n; $i++) { $x = $w[$i]; $s += loud($i) + $x; } return $s; }
function sum_weird($n) { $s = 0.0; for ($i = 0; $i < $n; $i++) { $s += weird($i) + 1.0; } return $s; }
function sum_boom($n) { $s = 0.0; for ($i = 0; $i < $n; $i++) { $s += boom($i); } return $s; }
function plain_int($n) { $s = 0.0; for ($i = 0; $i < $n; $i++) { $s = val(1) + 1.5; } return $s; }
for ($r = 0; $r < 3; $r++) {
    $calls = 0;
    echo json_encode(sum_val(10)), " calls=$calls\n";
    try { echo json_encode(sum_loud(4, [1.0, 2.0, 3.0, 4.0])); sum_loud(4, [1.0, 2.0, "x", 3.0]); } catch (TypeError $e) { echo " TypeError"; } echo "\n";
    try { echo json_encode(sum_weird(5)), "\n"; } catch (TypeError $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { echo sum_boom(5), "\n"; } catch (RuntimeException $e) { echo 'caught ', $e->getMessage(), "\n"; }
    echo json_encode(plain_int(3)), "\n";
}
