<?php
class A {
    public $pub = 1;
    protected $prot = 2;
    private $priv = 3;
    public ?int $typed;
    public $n = null;
    public function look($o) {
        $r = [];
        $r[] = $o->pub ?? 'd';
        $r[] = $o->prot ?? 'd';
        $r[] = $o->priv ?? 'd';
        $r[] = $o->typed ?? 'd';
        $r[] = $o->n ?? 'd';
        $r[] = isset($o->pub) ? 'y' : 'n';
        $r[] = isset($o->priv) ? 'y' : 'n';
        $r[] = empty($o->pub) ? 'e' : 'f';
        $r[] = empty($o->n) ? 'e' : 'f';
        return implode(',', $r);
    }
}
class B extends A {
    private $priv = 'b';
    public function lookB($o) { return ($o->priv ?? 'd') . '/' . (isset($o->priv) ? 'y' : 'n'); }
}
class M {
    public $real = 'r';
    public function __isset($n) { return $n === 'virt'; }
    public function __get($n) { return "magic:$n"; }
}
function outside($o) {
    return ($o->pub ?? 'd') . ',' . ($o->prot ?? 'd') . ',' . ($o->priv ?? 'd') . ',' . (isset($o->prot) ? 'y' : 'n');
}
function mag($o) {
    return ($o->real ?? 'd') . ',' . ($o->virt ?? 'd') . ',' . ($o->none ?? 'd') . ',' . (isset($o->virt) ? 'y' : 'n') . ',' . (empty($o->virt) ? 'e' : 'f');
}
$a = new A; $b = new B; $m = new M;
for ($i = 0; $i < 3; $i++) {
    echo $a->look($a), ' | ', $a->look($b), ' | ', $b->lookB($b), ' | ', $b->lookB($a), "\n";
    echo outside($a), ' | ', outside($b), "\n";
    echo mag($m), "\n";
    $a->typed = $i;
    if ($i == 1) { unset($a->pub); unset($m->real); $a->n = 'set'; }
}
// shape changes: dynamic props, references
$objs = [];
for ($i = 0; $i < 4; $i++) {
    $o = new stdClass;
    if ($i % 2) $o->extra = $i;
    $o->v = $i;
    $objs[] = $o;
}
$x = 10;
$objs[] = $r = new stdClass; $r->v = &$x;
foreach ($objs as $o) echo $o->v ?? 'd', isset($o->extra) ? '+' : '-', ' ';
$x = 11;
foreach ($objs as $o) echo $o->v ?? 'd', ' ';
echo "\n";
function chain($o) { return $o->a->b ?? 'none'; }
$c = new stdClass; $c->a = new stdClass;
echo chain($c), ' ';
$c->a->b = 'deep'; echo chain($c), ' ';
unset($c->a); echo chain($c), "\n";
