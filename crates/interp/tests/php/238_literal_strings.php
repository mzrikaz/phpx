<?php
function f($i) {
    $s = 'abc';
    $s[0] = 'x';
    $s .= $i;
    $t = 'abc';
    $u = 'abc';
    $u .= '!';
    return "$s $t $u";
}
function g(&$r) { $r[1] = 'Z'; $r .= '?'; }
$out = [];
for ($i = 0; $i < 4; $i++) {
    $out[] = f($i);
    $k = 'key';
    g($k);
    $out[] = $k . '|' . 'key';
    $arr = ['lit' => 'val'];
    $arr['lit'] .= $i;
    $out[] = $arr['lit'] . '|' . 'val';
    $x = 'hello';
    $y = $x;
    $y[0] = 'J';
    $out[] = "$x $y " . strtoupper('hello') . ' ' . str_repeat('ab', 2);
}
echo implode("\n", $out), "\n";
