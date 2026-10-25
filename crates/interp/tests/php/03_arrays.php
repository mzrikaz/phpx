<?php
$a = [3 => 'c', 1 => 'a', 2 => 'b'];
$a[] = 'd';
var_dump($a, count($a), array_keys($a), array_values($a));
$b = ["x" => 1, "5" => 2, "05" => 3, true => 4, null => 5, 1.7 => 6];
var_dump($b);
$n = [-5 => 'a'];
$n[] = 'b';
var_dump($n);
var_dump(array_merge([1, 2], ['a' => 1], [3]), [1, 2] + [5, 6, 7], array_combine(['a', 'b'], [1, 2]));
var_dump(array_slice([1, 2, 3, 4, 5], 1, 3), array_slice(['a' => 1, 5 => 2, 3], 1), array_slice([1, 2, 3], -2, 1, true));
$arr = [1, 2, 3, 4, 5];
$removed = array_splice($arr, 1, 2, ['x', 'y', 'z']);
var_dump($arr, $removed);
var_dump(array_map(fn($x) => $x * 2, [1, 2, 3]), array_map(fn($x) => $x, ['k' => 'v']), array_map(null, [1, 2], ['a', 'b']));
var_dump(array_filter([1, 0, 2, null, 3, '', '0', 'a']), array_filter(['a' => 1, 'b' => 2, 'c' => 3], fn($k) => $k != 'b', ARRAY_FILTER_USE_KEY));
var_dump(array_reduce([1, 2, 3], fn($c, $i) => $c + $i, 10), in_array("1", [1, 2]), in_array("1", [1, 2], true), array_search(2, [1, 2, 3]));
var_dump(array_unique([1, "1", 2, 2.0, "a", "A"]), array_reverse(['a' => 1, 2, 3]), array_reverse([1, 2, 3], true), array_flip(['a', 'b']));
var_dump(range(1, 5), range(0, 10, 3), range(5, 1, 2), range('a', 'e'), range(0, 1, 0.25), array_fill(5, 3, 'x'), array_fill_keys(['a', 'b'], 0));
var_dump(array_column([['id' => 1, 'n' => 'a'], ['id' => 2, 'n' => 'b']], 'n', 'id'), array_chunk([1, 2, 3, 4, 5], 2), array_chunk(['a' => 1, 'b' => 2], 1, true));
var_dump(array_diff([1, 2, 3, 4], [2, 4]), array_intersect([1, 2, 3], [2, 3, 5]), array_diff_key(['a' => 1, 'b' => 2], ['a' => 9]), array_intersect_key(['a' => 1, 'b' => 2], ['b' => 0]));
var_dump(array_key_first(['x' => 1, 'y' => 2]), array_key_last([1, 2, 3]), array_is_list([1, 2]), array_is_list([1 => 1]), array_count_values(['a', 'b', 'a', 1, '1']));
var_dump(array_pad([1, 2], 4, 0), array_pad([1, 2], -4, 0), array_key_exists('a', ['a' => null]), isset(['a' => null]['a']));
$s = [5, 3, 8, 1]; sort($s); var_dump($s);
$s = [5, 3, 8, 1]; rsort($s); var_dump($s);
$s = ['b' => 2, 'a' => 3, 'c' => 1]; asort($s); var_dump($s);
$s = ['b' => 2, 'a' => 3, 'c' => 1]; arsort($s); var_dump($s);
$s = ['b' => 2, 'a' => 3, 'c' => 1]; ksort($s); var_dump($s);
$s = ['b' => 2, 'a' => 3, 'c' => 1]; krsort($s); var_dump($s);
$s = [[2, 'b'], [1, 'a'], [2, 'a']]; usort($s, fn($x, $y) => $x[0] <=> $y[0]); var_dump($s);
$s = ['x' => 3, 'y' => 1]; uasort($s, fn($a, $b) => $a <=> $b); var_dump($s);
$s = ["10", 9, "9a", 1.5, true]; sort($s); var_dump($s);
$s = ["img12.png", "img10.png", "img2.png"]; natsort($s); var_dump($s);
$q = [1, 2]; array_push($q, 3, 4); $last = array_pop($q); $first = array_shift($q); array_unshift($q, 0); var_dump($q, $last, $first);
// Copy-on-write value semantics.
$orig = ['a' => [1, 2]];
$copy = $orig;
$copy['a'][] = 3;
$copy['b'] = 'new';
var_dump($orig, $copy);
function addOne(array $x) { $x[] = 1; return $x; }
$in = [0]; $out = addOne($in); var_dump($in, $out);
// Nested auto-vivification and unset.
$tree = [];
$tree['a']['b']['c'] = 1;
$tree['a']['list'][] = 'x';
$tree['a']['list'][] = 'y';
unset($tree['a']['b']);
var_dump($tree);
[$p, [$q2, $r]] = [1, [2, 3]];
['k1' => $k1, 'k2' => $k2] = ['k2' => 'two', 'k1' => 'one'];
[, $second] = [1, 2];
var_dump($p, $q2, $r, $k1, $k2, $second);
foreach ([[1, 2], [3, 4]] as [$x, $y]) { echo "$x-$y "; }
echo "\n";
$arr = [1, 2, 3];
foreach ($arr as $k => $v) { if ($k == 0) { $arr[] = 99; } echo $v, " "; }
echo count($arr), "\n";
var_dump([...[1, 2], ...['a' => 'b'], 3], array_sum([]), max([1, 5, 3]), array_find([1, 5, 8], fn($x) => $x > 3), array_any([1, 2], fn($x) => $x > 1));
