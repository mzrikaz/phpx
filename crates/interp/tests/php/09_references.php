<?php
// By-reference parameters: scalars, arrays, elements, properties.
function inc(&$n, $by = 1) { $n += $by; }
function push(array &$list, ...$items) { foreach ($items as $i) { $list[] = $i; } }
$x = 1; inc($x); inc($x, 10);
$list = []; push($list, 'a', 'b');
$arr = ['k' => 1]; inc($arr['k']); inc($arr['new']);
class Box { public $v = 5; public array $items = []; }
$b = new Box; inc($b->v); push($b->items, 'x');
var_dump($x, $list, $arr, $b->v, $b->items);

// Laravel's data_set, verbatim style.
function data_set_simple(&$target, $key, $value) {
    $segments = is_array($key) ? $key : explode('.', $key);
    $segment = array_shift($segments);
    if ($segments) {
        if (!isset($target[$segment])) { $target[$segment] = []; }
        data_set_simple($target[$segment], $segments, $value);
    } else {
        $target[$segment] = $value;
    }
    return $target;
}
$config = ['app' => ['name' => 'x']];
data_set_simple($config, 'app.name', 'phpx');
data_set_simple($config, 'db.connections.mysql.host', 'localhost');
var_dump($config);

// foreach by reference, including the classic trap.
$nums = [1, 2, 3];
foreach ($nums as &$n) { $n *= 10; }
var_dump($nums);
foreach ($nums as $n) {}          // $n still references $nums[2]
var_dump($nums);                  // [10, 20, 20]
unset($n);
$rows = [['q' => 1], ['q' => 2]];
foreach ($rows as $i => &$row) { $row['q'] += $i; $row['seen'] = true; }
unset($row);
var_dump($rows);
$grow = [1];
foreach ($grow as $k => &$g) { if ($k < 2) { $grow[] = $k + 10; } }
unset($g);
var_dump($grow);

// Assignment by reference.
$a = 1; $r = &$a; $r = 2; var_dump($a);
$data = ['x' => ['y' => 1]];
$ref = &$data['x']['y']; $ref = 'changed'; var_dump($data);
$copy = $data;                    // ref with refcount 2 stays shared in the copy
$ref = 'again';
var_dump($data['x']['y'], $copy['x']['y']);
unset($ref);
$copy2 = $data;                   // now refcount 1: copy gets a plain value
$copy2['x']['y'] = 'only copy';
var_dump($data['x']['y'], $copy2['x']['y']);
$p = 'one'; $q = 'two';
$pair = [&$p, &$q];
$pair[0] = 'ONE';
var_dump($p, $pair);
$obj = new Box; $alias = &$obj->v; $alias = 99; var_dump($obj->v);

// References in closures and statics.
$total = 0;
$add = function ($n) use (&$total) { $total += $n; };
array_map($add, [1, 2, 3]);
var_dump($total);
function &counterRef() { static $c = 0; $c++; return $c; }
counterRef(); var_dump(counterRef());
// Swapping and recursion with accumulators.
function swap(&$l, &$r) { [$l, $r] = [$r, $l]; }
$s1 = 'L'; $s2 = 'R'; swap($s1, $s2); var_dump($s1, $s2);
function flatten(array $in, array &$out = []) { foreach ($in as $v) { is_array($v) ? flatten($v, $out) : $out[] = $v; } return $out; }
var_dump(flatten([1, [2, [3, 4]], 5]));
// Typed by-ref params coerce the caller's variable.
function toInt(int &$v) { $v++; }
$str = "41"; toInt($str); var_dump($str);
// Sorting built-ins on references-holding arrays.
$vals = [3, 1, 2]; $vr = &$vals[0]; sort($vals); var_dump($vals); unset($vr);
