<?php
function f($a = 1, $b = 2, $c = 3) { return [func_num_args(), func_get_args()]; }
var_dump(f(), f(5), f(b: 7), f(1, c: 9), f(c: 1));
function g(...$xs) { return func_num_args(); } var_dump(g(1, 2), g(a: 1));
