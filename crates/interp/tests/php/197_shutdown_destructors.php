<?php
// Objects still alive at shutdown get __destruct, in handle order.
class D {
    public $peer;
    function __construct(public $n) {}
    function __destruct() { echo "destruct {$this->n}\n"; }
}
class Holder { static $one; static $list = []; }
$a = new D("cycle-a");
$b = new D("cycle-b");
$a->peer = $b;
$b->peer = $a;
unset($a, $b);
Holder::$one = new D("static");
Holder::$list[] = new D("in-array");
$g = new D("global");
function f() { static $s; $s = new D("function-static"); }
f();
register_shutdown_function(function () { echo "shutdown function\n"; });
echo "end\n";
