<?php
class A { public $p; public $arr = []; }
function v($x) { echo "eval $x\n"; return $x; }
function t($o) { $o->p ??= v(1); $o->p ??= v(2); $o->arr['k'] ??= v(3); $o->arr['k'] ??= v(4); $o->arr['n'][] = 1; var_dump($o->p, $o->arr); $o->arr['z'] ??= null; var_dump(array_key_exists('z', $o->arr)); return $o->arr['k'] ??= 9; }
for ($i = 0; $i < 3; $i++) { var_dump(t(new A)); }
