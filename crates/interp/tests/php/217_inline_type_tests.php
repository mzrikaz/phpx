<?php
namespace App {
    // Unqualified calls in a namespace fall back to the global built-ins.
    function check($v) { return [is_null($v), is_array($v), is_string($v), is_int($v), is_bool($v), is_float($v), is_object($v), is_integer($v), is_double($v)]; }
    function size($v) { try { return count($v); } catch (\TypeError $e) { return $e->getMessage(); } }
    function len($v) { return strlen($v); }
    foreach ([null, [1], 'a', 1, true, 1.5, new \ArrayObject([1, 2, 3])] as $v) echo json_encode(check($v)), "\n";
    echo size([1, 2]), ' ', size(new \ArrayObject([1, 2, 3])), ' ', size('x'), "\n";
    echo len('abc'), ' ', len(12345), ' ', len(1.5), "\n";
    $r = [1, 2, 3]; $ref = &$r; echo count($ref), ' ', is_array($ref) ? 'y' : 'n', "\n";
}
namespace Shadow {
    function is_null($v) { return 'shadowed'; }
    echo is_null(null), "\n";
}
