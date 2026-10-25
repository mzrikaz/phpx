<?php
namespace App;
function f() { echo "[f]"; return [1, 2, 3]; }
class Cnt implements \Countable { function count(): int { return 2; } }
function g() { echo "[g]"; return new Cnt; }
function t($x) { return [is_array($x) ? -1 : strlen($x ?? ''), count((array)$x), is_array($x), is_string($x), is_int($x), is_float($x), is_bool($x), is_null($x), is_object($x), is_scalar($x)]; }
for ($i = 0; $i < 3; $i++) {
  echo json_encode(t("abc")), json_encode(t([1, 2])), json_encode(t(5)), json_encode(t(1.5)), json_encode(t(null)), json_encode(t(true)), "\n";
  echo count(f()), count(g()), \count(['a' => 1, 'b' => 2]), sizeof([]), strlen(123), strlen(1.5), "\n";
  try { echo count(5); } catch (\TypeError $e) { echo "TE: ", $e->getMessage(), "\n"; }
  $h = []; for ($k = 0; $k < 5; $k++) { $h["k$k"] = $k; } unset($h['k1']); echo count($h), " ", count([1, [2, 3]], COUNT_RECURSIVE), "\n";
}
