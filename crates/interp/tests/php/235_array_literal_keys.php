<?php
// Array literals with constant keys: key order, numeric-string keys, the
// next free index, values evaluated left to right, copies independent.
function build($i) {
    $x = 'v';
    $a = ['k' => $i, 'v' => $x, '7' => 'seven', 3 => 'three', 'last'];
    $b = [$i, $i + 1, 'x'];
    $c = [5 => 'a', 'b', 'k' => 'c', 'd'];
    $n = 0;
    $d = ['first' => ++$n, 'second' => ++$n, 'third' => $n * 10];
    $e = ['-0' => 1, '-5' => 2, '08' => 3, ' 1' => 4];
    return [$a, $b, $c, $d, $e];
}
for ($i = 0; $i < 3; $i++) {
    $r = build($i);
    $r[0]['k'] = 'changed';
    $r[2][] = 'pushed';
}
var_dump(build(9));
var_dump($r);
$base = ['a' => 1, 'b' => [1, 2], 'c' => 'str'];
for ($i = 0; $i < 3; $i++) { $copy = $base; $copy['d'] = $i; $copy['a'] = -$i; }
var_dump($base, $copy);
$x = 1;
$withRef = ['r' => &$x, 'p' => 2];
$cp = $withRef;
$cp['p'] = 3;
$x = 5;
echo json_encode([$withRef, $cp]), "\n";
