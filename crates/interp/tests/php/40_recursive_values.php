<?php
$a = []; $a[] = &$a; $a['x'] = 1;
var_dump($a);
print_r($a); echo "\n";
echo @count($a, COUNT_RECURSIVE), "\n";
echo @var_export($a, true), "\n";
var_dump(json_encode($a), json_last_error(), json_last_error_msg());
var_dump($a == $a, $a === $a);
$b = [$a, $a]; sort($b); echo count($b), "\n";
try { array_walk_recursive($a, fn() => 1); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { array_replace_recursive($a, $a); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$o = new stdClass; $o->me = $o; $o->list = [$o];
var_dump($o); print_r($o); echo "\n";
echo @var_export($o, true), "\n";
var_dump(json_encode($o), json_last_error_msg());
$x = [1, [2, [3]]]; $y = [$x, $x];
var_dump(json_encode($y), count($y, COUNT_RECURSIVE));
print_r($y);
