<?php
class Svc {
    public $log = [];
    function get($k, $d = null) { return "$k/" . var_export($d, true); }
    function many(...$xs) { return implode(',', $xs); }
    function ref(&$x) { $x++; return $x; }
    private function hidden() { return 'hidden'; }
    protected function prot() { return 'prot'; }
    function inside($m, $a) { return $this->$m(...$a); }
}
class Magic extends Svc {
    function __call($n, $a) { return "call:$n(" . implode(',', $a) . ")"; }
    static function __callStatic($n, $a) { return "static:$n(" . implode(',', $a) . ")"; }
}
class F {
    static $inst;
    static function __callStatic($m, $args) { return static::$inst->$m(...$args); }
}
F::$inst = new Svc;
$o = new Svc; $mg = new Magic;
for ($i = 0; $i < 3; $i++) {
    $a = ['a', $i];
    echo $o->get(...$a), ' ', $o->get(...['x']), ' ', $o->many(...[1, 2, 3], ...[4]), "\n";
    $m = 'get'; $M = 'GET';
    echo $o->$m(...$a), ' ', $o->$M(...$a), ' ', $o->$m('p', ...[$i]), "\n";
    echo F::get('k', $i), ' ', F::many(...[$i, $i + 1]), "\n";
    echo $mg->hidden(1, 2), ' ', $mg->prot(...[3]), ' ', $mg->nope(...$a), ' ', Magic::stat(...$a), "\n";
    echo $o->inside('hidden', []), ' ', $o->inside('prot', []), "\n";
    echo $o->get(...['k' => 'named', 'd' => $i]), ' ', $o->get(...['d' => 7, 'k' => 'n2']), "\n";
    $v = 1; echo $o->ref($v), $v, "\n";
    try { $o->hidden(...[]); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { $n = null; $n->get(...$a); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { $o->missing(...$a); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { $o->get(...[]); } catch (ArgumentCountError $e) { echo get_class($e), "\n"; }
}
