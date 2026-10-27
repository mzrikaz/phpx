<?php
class P {
    function s(string $x) { return var_export($x, true); }
    function i(int $x) { return var_export($x, true); }
    function f(float $x) { return var_export($x, true); }
    function b(bool $x) { return var_export($x, true); }
    function a(array $x) { return count($x); }
    function ns(?string $x) { return var_export($x, true); }
    function nd(int $x = null) { return var_export($x, true); }
    function mix(int $a, $b, string $c, mixed $d) { return "$a|$b|$c|" . gettype($d); }
}
function free(int $a, ?array $b) { return $a . ':' . ($b === null ? 'null' : count($b)); }
$p = new P;
for ($k = 0; $k < 4; $k++) {
    echo $p->s('x'), $p->s(5), $p->s(1.5), $p->s(true), "\n";
    echo $p->i(3), $p->i('7'), $p->i(2.0), $p->i(false), "\n";
    echo $p->f(1.5), $p->f(2), $p->f('3.5'), "\n";
    echo $p->b(true), $p->b(0), $p->b('x'), "\n";
    echo $p->a([1, 2]), $p->ns(null), $p->ns('y'), $p->nd(null), $p->nd(4), "\n";
    echo $p->mix(1, [], 'c', new P), ' ', $p->mix('2', null, 3, null), ' ', free(1, null), ' ', free('5', [1]), "\n";
    $v = 'ref'; $r = &$v; echo $p->s($r), "\n";
    foreach ([[$p, 'i', 'abc'], [$p, 's', []], [$p, 'a', 'x'], [$p, 'b', []], [$p, 'ns', 1.5], ['free', null, null]] as [$o, $m, $arg]) {
        try {
            echo $m === null ? free($arg, null) : $o->$m($arg), "\n";
        } catch (TypeError $e) {
            echo get_class($e), ': ', explode(', called', $e->getMessage())[0], "\n";
        }
    }
}
