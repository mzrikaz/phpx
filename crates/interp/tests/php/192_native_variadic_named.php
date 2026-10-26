<?php
const D = 7;
class K { const X = 'kx'; public function m($a, $b = self::X, ...$rest) { return [$a, $b, $rest]; } public static function s(string $a = 'sa', int $b = D, $c = null) { return [$a, $b, $c]; } }
function v(...$all) { return $all; }
function v2($a, ...$more) { return [$a, $more]; }
function n($a, $b = 2, $c = 3, $d = D) { return [$a, $b, $c, $d, func_num_args()]; }
function tv(int ...$n) { return array_sum($n); }
function r(&$x, ...$rest) { $x++; return $rest; }
for ($i = 0; $i < 1500; $i++) {
  $o = [v(), v(1), v(1, 2, 3), v2(1), v2(1, 2, 3), call_user_func('v', 4, 5), v(...[6, 7]), v2(...['a' => 1, 'b' => 2]),
    n(1, c: 9), n(a: 1), n(d: 4, a: 0), n(1, 2, d: 5), call_user_func_array('n', ['a' => 1, 'd' => 2]),
    (new K)->m(1), (new K)->m(1, 2, 3, 4), (new K)->m(b: 5, a: 6), K::s(), K::s(b: 3), K::s(c: 'z'), tv(1, 2, 3)];
  $x = 1; r($x, 1, 2); $o[] = $x;
  if ($i % 500 == 0) var_dump($o);
}
foreach ([fn() => n(b: 1), fn() => n(1, a: 2), fn() => n(1, zz: 2), fn() => tv(1, 'x'), fn() => v(...['a' => 1], ...[2])] as $f) { try { var_dump($f()); } catch (Error $e) { echo get_class($e), ': ', str_replace(__FILE__, 'FILE', $e->getMessage()), "\n"; } }
