<?php
interface Shape { function area(): float; }
function make($kind, $n = null) {
    if ($kind === 0) {
        return new class { public $hits = 0; function __call($m, $a) { return "$m:" . count($a); } function __toString(): string { return "anon"; } };
    }
    if ($kind === 1) {
        return new class($n) implements Shape { function __construct(private float $r) {} function area(): float { return 3.0 * $this->r * $this->r; } };
    }
    $x = 5;
    return new class($x) { public $v; function __construct(&$v) { $this->v = $v; $v = 99; } };
}
$t = 0; $names = [];
for ($i = 0; $i < 3000; $i++) {
    $a = make(0); $t += strlen($a->foo(1, 2));
    $b = make(1, $i % 7); $t += (int) $b->area();
    $c = make(2); $t += $c->v;
    $names[get_class($a) === get_class(make(0)) ? 'same' : 'diff'] = 1;
}
echo $t, " ", (string) $a, " ", $b instanceof Shape ? "shape" : "no", " ", implode(",", array_keys($names)), "\n";
var_dump(str_contains(get_class($b), "class@anonymous"), (new ReflectionClass($c))->isAnonymous());
