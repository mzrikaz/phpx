<?php
// $a[k1][k2]... = v: through arrays, creating missing levels.
function nested() {
    $a = ['x' => ['y' => 1], 'n' => null, 's' => 'str', 'i' => 5, 7 => [8 => [9 => 'deep']]];
    $a['x']['z'] = 2;
    $a['new']['level'] = 3;
    $a['new2']['a']['b']['c'] = 4;
    $a[7][8][9] = 'replaced';
    $a["7"]["8"][10] = 'int-like';
    $a['n']['from-null'] = 5;
    try { $a['s']['bad'] = 6; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    try { $a['i']['bad'] = 7; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    $b = $a; $b['x']['y'] = 'copy'; // copy on write
    $r = &$a['x']; $a['x']['via'] = 'ref'; unset($r);
    $c = []; foreach (['ab', 'cd', 'ab', 'e'] as $i => $w) { $c[strlen($w)][$w] = $i; }
    $d = []; $v = $d[1][2] = 'chained';
    echo json_encode([$a, $b['x'], $c, $d, $v]), "\n";
    $u = null; $u['k']['j'] = 1; echo json_encode($u), "\n";
}
for ($i = 0; $i < 3; $i++) nested();
