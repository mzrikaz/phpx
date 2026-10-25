<?php
class N { public $o; public $n; function __construct($n) { $this->n = $n; } }
class D { public $o; public $n; function __construct($n) { $this->n = $n; } function __destruct() { echo "~D{$this->n} "; } }
$a = new N(1); $b = new N(2); $a->o = $b; $b->o = $a; unset($a, $b);
gc_collect_cycles(); echo "ok\n";
$x = new D(1); $y = new D(2); $x->o = $y; $y->o = $x; unset($x, $y);
echo "before ";
gc_collect_cycles(); echo "ok\n";
$keep = new N(3); $keep->o = $keep; echo gc_collect_cycles(), " ", $keep->n, "\n";
$arr = [new N(4)]; $arr[0]->o = &$arr; unset($arr); gc_collect_cycles(); echo "ok\n";
$c = new N(5); $c->o = function () use ($c) { return $c->n; }; $f = $c->o; unset($c); echo $f(), " ", gc_collect_cycles(), "\n"; unset($f); gc_collect_cycles(); echo "ok\n";
var_dump(gc_enabled());
