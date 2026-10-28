<?php
class T { public float $f = 0.0; public int $i = 0; public string $s = ''; public ?int $n = null; public array $a = []; public bool $b = false; public readonly int $r;
    function __construct() { $this->r = 1; } }
function w(T $t, $f, $i, $s) { $t->f = $f; $t->i = $i; $t->s = $s; $t->n = $i; $t->n = null; $t->a = [$i]; $t->b = true; return [$t->f, $t->i, $t->s, $t->n, $t->a, $t->b]; }
$t = new T;
for ($k = 0; $k < 5; $k++) {
    echo json_encode(w($t, 1.5, 2, 'x')), json_encode(w($t, 3, 4, 'y')), "\n";
    foreach ([['f', 'abc'], ['i', 1.5], ['i', '7'], ['s', 12], ['s', []], ['a', 'no'], ['b', 0], ['r', 5]] as [$p, $v]) {
        try { $t->$p = $v; echo $p, '=', json_encode($t->$p), ' '; } catch (TypeError | Error $e) { echo get_class($e), ' '; }
    }
    echo "\n";
}
