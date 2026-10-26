<?php
// Writes, references and unset() of private/protected properties from
// outside the class: "Cannot access ..." (interpreted and compiled code).
class B { private $secret = []; protected $prot = []; public $pub = []; }
class M { private $data = ['items' => []]; function &__get($n) { echo "__get $n\n"; return $this->data[$n]; } }
class U { private $s = 1; function __unset($n) { echo "__unset $n\n"; } }
$b = new B;
$cases = [
    fn() => $b->secret = 1,
    fn() => $b->secret['x'] = 1,
    fn() => $b->secret[] = 1,
    fn() => $b->secret['x']['y'] = 1,
    fn() => $b->prot['x'] = 1,
    fn() => $b->prot = 1,
    fn() => $b->secret['x'] .= 'a',
    fn() => $b->secret['x']++,
    fn() => $b->prot++,
    fn() => $b->secret .= 'x',
    function () use ($b) { unset($b->secret['x']); },
    function () use ($b) { $r = &$b->secret; },
    function () use ($b) { $r = &$b->secret['x']; },
    function () use ($b) { foreach ($b->secret as &$v); },
    function () use ($b) { sort($b->secret); },
    function () use ($b) { $b->pub['ok'] = 1; $r = &$b->pub['r']; unset($b->pub['ok']); },
    function () use ($b) { unset($b->secret); },
    function () use ($b) { unset($b->prot); },
];
// Each case twice in a hot loop, so compiled code takes them too.
function run($f) { try { $f(); return "ok"; } catch (Error $e) { return $e->getMessage(); } }
$seen = [];
for ($round = 0; $round < 1500; $round++) {
    foreach ($cases as $i => $f) {
        $r = run($f);
        if (!isset($seen[$i]) || $seen[$i] !== $r) { echo "$round $i $r\n"; $seen[$i] = $r; }
    }
}
var_dump($b);
// With &__get, an indirect write goes through the returned reference.
$m = new M;
function mset($m, $i) { $m->items[$i] = $i; }
for ($i = 0; $i < 3; $i++) mset($m, $i);
print_r($m);
// With __unset, unsetting an inaccessible property calls it.
$u = new U;
function uns($u) { unset($u->s); }
for ($i = 0; $i < 2; $i++) uns($u);
// unset() of an element of a missing property creates it as null.
$o = new stdClass;
function unsdim($o) { unset($o->a['b']); }
unsdim($o);
var_dump($o);
echo "end\n";
