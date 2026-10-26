<?php
$a = ['x' => 1, 'y' => 2];
$b = array_merge($a, ['z' => 3]);
$b['x'] = 9;
var_dump($a, $b);
var_dump(array_merge($a, $a), array_merge(['k' => 'v'], $a, [5, 6], ['y' => 'Y']));
var_dump(array_merge([3 => 'a', 7 => 'b'], ['q' => 1, 9 => 'c']), array_merge(), array_merge([]), array_merge([], ['a' => 1]));
$r = 1; $withRef = ['r' => &$r, 's' => 2];
$m = array_merge($withRef, ['t' => 3]); $r = 100; var_dump($m);
function casts($key) { return array_merge([$key => 'int'], ['a' => 'string', 'b' => 'bool']); }
for ($i = 0; $i < 1500; $i++) $c = casts('id');
var_dump($c, casts('a'));
function lit($k, $v) { return [$k => $v, 'fixed' => 1, 2 => 'two', 'x']; }
for ($i = 0; $i < 1500; $i++) $l = lit('dyn', $i);
var_dump($l, lit(0, 'zero'), lit('5', 'five'), array_is_list(lit(0, 'a')));
try { array_merge($a, 'nope'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$x = ['p' => 1]; $y = &$x; var_dump(array_merge($y, ['q' => 2]), $x);
