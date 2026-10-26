<?php
// Call sites answer trivial getters without calling them: results must
// follow the object, its class, visibility and property state.
#[AllowDynamicProperties]
class Base {
    protected $key = 'id'; private $secret = 'base-secret'; public $n = 0; public ?int $typed;
    function key() { return $this->key; } function secret() { return $this->secret; }
    function n() { return $this->n; } function typed() { return $this->typed; }
    function lit() { return 'L'; } function drop() { unset($this->key); } function nul() { return null; } function num() { return 4.5; }
}
class Child extends Base { private $secret = 'child-secret'; protected $key = 'uuid'; function lit() { return 'C'; } }
class Magic extends Base { function __get($p) { return "magic:$p"; } }
function run($o) { return [$o->key(), $o->secret(), $o->n(), $o->lit(), $o->nul(), $o->num()]; }
$objs = [new Base, new Child, new Magic];
for ($i = 0; $i < 2500; $i++) {
    foreach ($objs as $o) { $o->n = $i; $r = run($o); }
}
foreach ($objs as $o) var_dump(run($o));
$b = new Base;
for ($i = 0; $i < 1500; $i++) { $b->n = $i; $x = $b->n(); }
var_dump($x);
$b->dyn = 1;               // new shape
var_dump($b->n());
unset($b->n);              // gone: warning, null
var_dump(@$b->n());
$m = new Magic; for ($i = 0; $i < 1500; $i++) $z = $m->key(); $m->drop(); var_dump($z, $m->key());
$r = 5; $c = new Base; $c->n = &$r; $r = 6; var_dump($c->n());
try { var_dump((new Base)->typed()); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$t = new Base; $t->typed = 3; for ($i = 0; $i < 1500; $i++) $y = $t->typed(); var_dump($y);
$anon = new class extends Base { function n() { return 'anon'; } }; var_dump(run($anon)[2]);
