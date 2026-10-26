<?php
class G { function __call($n, $a) { return "call:$n(" . implode(',', $a) . ")"; } static function __callStatic($n, $a) { return "static:$n(" . implode(',', $a) . ")"; } function real() { return 'real'; } }
$g = new G;
var_dump(call_user_func_array([$g, 'safeEmail'], [1, 2]), call_user_func([$g, 'real']), call_user_func('G::foo', 3), call_user_func(['G', 'bar']), array_map([$g, 'up'], ['a']));
$c = Closure::fromCallable([$g, 'magic']); var_dump($c('x'));
var_dump(is_callable([$g, 'whatever']), is_callable('G::x'));
class H { function t() { return call_user_func([static::class, 'thing']); } function __call($n, $a) { return "inst:$n"; } }
var_dump((new H)->t());
