<?php
const K = 'konst';
class D {
    const C = 'cc';
    function m($a, $b = null, $c = 5, $d = 'str', $e = [], $f = -3, $g = 1.5, $h = true, $i = K, $j = self::C, $k = [1, 'x' => 2], $l = -2.5) {
        $e[] = $a;
        $d .= '!';
        return json_encode([$a, $b, $c, $d, $e, $f, $g, $h, $i, $j, $k, $l]);
    }
}
function typed(int $n, string $s = 'dflt', ?array $a = null, float $f = 2, bool ...$rest) {
    return json_encode([$n, $s, $a, $f, $rest]);
}
$o = new D;
for ($r = 0; $r < 3; $r++) {
    echo $o->m(1), "\n", $o->m(2, 'b'), "\n", $o->m(3, 'b', 6, 'x', [9]), "\n";
    echo typed(1), "\n", typed(2, 'z'), "\n", typed(3, 'z', [1], 4, true, false), "\n";
    echo $o->m(...[4]), ' ', $o->m(4, e: ['named']), "\n";
}
