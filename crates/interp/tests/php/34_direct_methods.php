<?php
class Base { protected $v; function __construct($v) { $this->v = $v; } function get() { return $this->v; } private function secret() { return "base-secret"; } function callSecret() { return $this->secret(); } function self() { return $this; } }
class Kid extends Base { function get() { return "kid:" . parent::get(); } private function secret() { return "kid-secret"; } }
class D { public $n; function __construct($n) { $this->n = $n; echo "make $n\n"; } function hi() { return "hi {$this->n}"; } function __destruct() { echo "bye {$this->n}\n"; } }
class T { function boom($i) { if ($i % 2) throw new Exception("boom $i"); return "ok $i"; } function chain() { return $this->self2()->self2(); } function self2() { return $this; } function rec($n) { return $n == 0 ? 0 : 1 + $this->rec($n - 1); } }
class Typed { function f(int $x): string { return "x$x"; } }
$objs = [new Base(1), new Kid(2), new Base(3), new Kid(4)];
for ($r = 0; $r < 3; $r++) {
    foreach ($objs as $o) { echo $o->get(), " ", $o->callSecret(), " ", get_class($o->self()), "\n"; }
    echo (new D($r))->hi(), "\n";
    echo "after\n";
    $t = new T;
    foreach ([1, 2] as $i) { try { echo $t->boom($i), "\n"; } catch (Exception $e) { echo $e->getMessage(), "@", $e->getLine(), "\n"; } }
    echo get_class($t->chain()), " ", $t->rec(50), "\n";
    $ty = new Typed;
    echo $ty->f(5), $ty->f("6"), "\n";
    try { $ty->f("z"); } catch (TypeError $e) { echo str_replace(__FILE__, 'F', $e->getMessage()), "\n"; }
    try { $n = null; $n->get(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
}
$d = new D(9);
$h = $d->hi();
$d = null;
echo $h, "\n";
