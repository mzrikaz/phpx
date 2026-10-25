<?php
print_r(array_change_key_case(['FoO' => 1, 'bar' => 2, 3 => 'x', 'BAR' => 4]));
print_r(array_change_key_case(['FoO' => 1, 'bar' => 2], CASE_UPPER));
$a = ['a' => 'green', 'b' => 'brown', 'c' => 'blue', 'red'];
$b = ['a' => 'GREEN', 'B' => 'brown', 'yellow', 'red'];
print_r(array_intersect_assoc($a, $b));
print_r(array_intersect_assoc(['a' => 1, 'b' => '2'], ['a' => '1', 'b' => 2], ['a' => 1]));
print_r(array_uintersect_assoc($a, $b, 'strcasecmp'));
print_r(array_intersect_uassoc($a, $b, 'strcasecmp'));
print_r(array_uintersect_uassoc($a, $b, 'strcasecmp', 'strcasecmp'));
print_r(array_diff_uassoc($a, $b, 'strcasecmp'));
print_r(array_udiff_assoc($a, $b, 'strcasecmp'));
print_r(array_udiff_uassoc($a, $b, 'strcasecmp', 'strcasecmp'));

$data = [3, 1, 2, 1];
$data2 = ['c', 'a', 'b', 'z'];
array_multisort($data, $data2);
print_r($data);
print_r($data2);
$ar1 = [10, 100, 100, 0];
$ar2 = [1, 3, 2, 4];
array_multisort($ar1, $ar2);
var_dump($ar1, $ar2);
$ar = [["10", 11, 100, 100, "a"], [1, 2, "2", 3, 1]];
array_multisort($ar[0], SORT_ASC, SORT_STRING, $ar[1], SORT_NUMERIC, SORT_DESC);
var_dump($ar);
$vol = ['x' => 67, 'y' => 86, 3 => 85, 'z' => 98, 7 => 86, 'w' => 67];
$ed = ['x' => 2, 'y' => 1, 3 => 6, 'z' => 2, 7 => 6, 'w' => 7];
array_multisort($vol, SORT_DESC, $ed, SORT_ASC);
print_r($vol);
print_r($ed);
$words = ['banana', 'Apple', 'cherry', 'apple'];
array_multisort($words, SORT_ASC, SORT_STRING | SORT_FLAG_CASE);
print_r($words);
try { $x = [1, 2]; $y = [1]; array_multisort($x, $y); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
