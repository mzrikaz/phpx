<?php
function casts($v) {
    return [(string)$v, (int)$v, (float)$v, (array)$v, (bool)$v];
}
function issets($a, $k) {
    $r = &$a;
    $copy = $a;
    return [isset($a[$k]), isset($r[$k]), isset($copy[$k]), isset($a[$k]) ? 'y' : 'n', empty($a[$k]), $a[$k] ?? 'dflt'];
}
class AA implements ArrayAccess {
    function offsetExists($o): bool { echo "exists($o) "; return $o === 'x'; }
    function offsetGet($o): mixed { return $o === 'x' ? null : 1; }
    function offsetSet($o, $v): void {}
    function offsetUnset($o): void {}
}
for ($i = 0; $i < 3; $i++) {
    foreach (['abc', '12', 7, 1.5, true, null] as $v) {
        echo json_encode(casts($v)), "\n";
    }
    $s = 'hello'; $t = (string)$s; $t[0] = 'J'; echo $s, $t, "\n";
    $arr = ['a' => 1, 'n' => null, 5 => 'five', '7' => 'seven', 'r' => 0];
    $ref = 1; $arr['ref'] = &$ref;
    foreach (['a', 'n', 5, '5', 7, '7', 'missing', 'r', 'ref', true] as $k) {
        echo json_encode([$k, issets($arr, $k)]), "\n";
    }
    $str = 'abc';
    var_dump(isset($str[1]), isset($str[5]), isset($str['1']), isset($str['x']));
    $o = new AA;
    var_dump(isset($o['x']), isset($o['y']));
    $n = null; var_dump(isset($n['a']));
    $unset_local = 1; unset($unset_local); var_dump(isset($unset_local['a']));
}
