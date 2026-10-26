<?php
function f($a, $b = 2) { $a = 9; return [func_get_args(), func_num_args()]; }
function g(...$r) { return [func_get_args(), func_num_args()]; }
function h($x, ...$r) { $x = 'X'; $r[] = 'added'; return [func_get_args(), func_num_args(), $r]; }
class C { function when($v = null, $cb = null) { if (func_num_args() === 1) return 'one'; return [func_num_args(), func_get_args()]; } static function s() { return func_get_args(); } }
for ($i = 0; $i < 3; $i++) {
  var_dump(f(1), f(1, 2, 3), g(), g(1, [2]), h(1, 2, 3), (new C)->when(5), (new C)->when(5, 6, 7), C::s('a', 'b'));
  var_dump(call_user_func('f', 4, 5, 6), (new C)->when(...[1, 2]), call_user_func_array([new C, 'when'], ['v' => 1]));
}
