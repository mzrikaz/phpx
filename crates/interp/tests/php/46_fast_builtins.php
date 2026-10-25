<?php
namespace App { function strpos($a, $b) { return "shadowed"; } function run() { return strpos("abc", "b"); } }
namespace {
function v($x) { return is_string($x) ? "s:" . bin2hex($x) : (is_array($x) ? json_encode(array_map(fn($e) => is_string($e) ? bin2hex($e) : $e, $x)) : var_export($x, true)); }
function run() {
    $out = [];
    $strs = ["", "a", "abc", "Hello World", "  pad \t\n", "0", "123", "-5", "1.5", "ÄbC"];
    foreach ($strs as $s) {
        $out[] = implode("|", [v(strlen($s) == 1 ? ord($s) : 0), v(str_contains($s, "l")), v(str_starts_with($s, "He")), v(str_ends_with($s, "ld")),
            v(strpos($s, "o")), v(substr($s, 1)), v(substr($s, -2, 1)), v(substr($s, 1, -1)), v(substr($s, 5, 2)), v(strtolower($s)), v(strtoupper($s)),
            v(trim($s)), v(str_replace("l", "L", $s)), v(ucfirst($s)), v(lcfirst($s)), v(str_repeat($s, 2)), v(explode("l", $s))]);
    }
    foreach ([0, 65, 255, 256, -1, 300] as $c) $out[] = v(chr($c));
    $arr = ['a' => 1, 'b' => null, 5 => 'x', '07' => 2, 'c' => [1]];
    foreach (['a', 'b', 'z', 5, '5', '07', 7, null, 'c'] as $k) $out[] = v(array_key_exists($k, $arr));
    $list = [1, '2', 3.0, 'abc', null, true, [1], '1e1'];
    foreach ([1, '1', 2, 3, '3', 0, 'abc', 'ABC', null, false, 10, [1], '01'] as $needle) $out[] = v(in_array($needle, $list)) . v(in_array($needle, $list, true));
    $out[] = implode(",", ["a", 1, "b"]) . "|" . implode("-", []) . "|" . implode(",", [1.5, true, null]);
    foreach ([5, -5, 0, 2.5, -2.5, PHP_INT_MIN] as $x) $out[] = v(abs($x)) . v(floor($x)) . v(ceil($x)) . v(intval($x));
    foreach ([[1, 2], [2, 1], [1, 1.0], [1.0, 1], [-1, -1.5], [3, 3]] as [$x, $y]) $out[] = v(min($x, $y)) . v(max($x, $y));
    $out[] = v(array_keys(['x' => 1, 3 => 2])) . v(array_values(['x' => 1, 3 => 2])) . v(array_key_first(['q' => 1, 'r' => 2])) . v(array_key_last(['q' => 1, 'r' => 2])) . v(array_key_first([]));
    $out[] = v(intval("12abc")) . v(intval(1e20)) . v(intval(NAN));
    return implode("\n", $out);
}
for ($i = 0; $i < 3; $i++) echo run(), "\n";
echo App\run(), "\n";
try { explode("", "a"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { str_repeat("a", -1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
}
