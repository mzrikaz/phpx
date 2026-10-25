<?php
// Built-in calls with run-time arguments (constant arguments get folded
// at compile time by PHP).
$strs = ["hello world", "HeLLo", "abc", "x"];
function b_ord($N, $s) { $c = ["a", "b", "c", "d"]; $r = 0; for ($i = 0; $i < $N; $i++) { $r += ord($c[$i & 3]); } return $r; }
function b_strpos($N, $s) { $r = 0; for ($i = 0; $i < $N; $i++) { $r += strpos($s[0], "w") ?: 0; } return $r; }
function b_substr($N, $s) { $r = ""; for ($i = 0; $i < $N; $i++) { $r = substr($s[0], $i & 3, 5); } return $r; }
function b_strtolower($N, $s) { $r = ""; for ($i = 0; $i < $N; $i++) { $r = strtolower($s[$i & 3]); } return $r; }
function b_in_array($N) { $a = [1, 2, 3, 4]; $r = 0; for ($i = 0; $i < $N; $i++) { $r += in_array($i & 7, $a) ? 1 : 0; } return $r; }
function b_key_exists($N) { $a = ['a' => 1, 'b' => 2]; $k = ['a', 'z']; $r = 0; for ($i = 0; $i < $N; $i++) { $r += array_key_exists($k[$i & 1], $a) ? 1 : 0; } return $r; }
function b_str_replace($N, $s) { $r = ""; for ($i = 0; $i < $N; $i++) { $r = str_replace("o", "0", $s[0]); } return $r; }
function b_implode($N) { $a = ["a", "b", "c"]; $r = ""; for ($i = 0; $i < $N; $i++) { $r = implode(",", $a); } return $r; }
function b_sprintf($N) { $r = ""; for ($i = 0; $i < $N; $i++) { $r = sprintf("%d-%s", $i, "x"); } return $r; }
function b_abs($N) { $r = 0; for ($i = 0; $i < $N; $i++) { $r += abs($i - 100); } return $r; }
$N = 2000000;
foreach (['b_ord' => 'ord', 'b_strpos' => 'strpos', 'b_substr' => 'substr', 'b_strtolower' => 'strtolower', 'b_in_array' => 'in_array', 'b_key_exists' => 'array_key_exists', 'b_str_replace' => 'str_replace', 'b_implode' => 'implode', 'b_sprintf' => 'sprintf', 'b_abs' => 'abs'] as $f => $d) {
    $t = microtime(true); $f($N, $strs); printf("%-18s %5.1f ns\n", $d, (microtime(true) - $t) * 1e9 / $N);
}
