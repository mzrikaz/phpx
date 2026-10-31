<?php
class P { public $a = 1; public $b = 2; public static $s = 5; static function twice($x) { return 2 * $x; } }
function poke(P $o, string $p, ?P $n) {
    $o->$p = $o->$p + 1;
    $m = 'twice';
    $t = P::$m($o->$p);
    $sp = 's';
    $t += P::$$sp;
    if (isset($o->$p)) $t++;
    $t += $n?->$p ?? 100;
    unset($o->zz);
    return $t;
}
$o = new P; $sum = 0;
for ($i = 0; $i < 3000; $i++) { $sum += poke($o, $i % 2 ? 'a' : 'b', $i % 3 ? $o : null); }
echo $sum, " ", $o->a, " ", $o->b, "\n";
