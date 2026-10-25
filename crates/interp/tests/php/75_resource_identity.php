<?php
$a = fopen('php://memory', 'r'); $b = fopen('php://memory', 'r');
$arr = [1 => $a, 2 => $b];
var_dump(array_search($a, $arr, true), array_search($b, $arr, true), $a === $a, $a === $b, $a !== $b, in_array($b, $arr, true), $a == $a, $a == $b);
