<?php
// `$obj->prop[k] = v` through the cached element-write path.
class Box_ {
    public $items = [];
    private $secret = [];
    public readonly array $ro;
    public $notArray = null;
    function __construct() { $this->ro = ['a' => 1]; }
    function put($k, $v) { $this->secret[$k] = $v; return $this; }
    function secret() { return $this->secret; }
    function fill($n) { for ($i = 0; $i < $n; $i++) $this->items["k$i"] = $i; }
}
class Sub extends Box_ { public $extra = []; }

function setItem($o, $k, $v) { $o->items[$k] = $v; }
function append($o, $v) { $o->items[] = $v; }

$a = new Box_; $b = new Sub;
for ($i = 0; $i < 3000; $i++) {           // hot enough to compile
    setItem($i % 2 ? $a : $b, $i % 5, $i);
    append($a, $i % 3);
}
echo count($a->items), " ", count($b->items), " ", $a->items[4], " ", $b->items[4], "\n";

// Numeric-string keys become ints, other keys stay strings.
setItem($a, "7", "seven"); setItem($a, "07", "oh-seven");
var_dump($a->items[7], $a->items["07"]);

// Copy-on-write: a copy taken before the write keeps the old value.
$copy = $a->items; setItem($a, 0, "new");
var_dump($copy[0] === "new", $a->items[0]);

// A reference into the array is written through.
$x = 1; $a->items['r'] = &$x; setItem($a, 'r', 99); var_dump($x);

// Private property: fine inside the class, error from outside.
for ($i = 0; $i < 2000; $i++) $a->put($i % 4, $i);
print_r($a->secret());
$bound = Closure::bind(function ($o) { for ($i = 0; $i < 2000; $i++) $o->secret['b'] = $i; }, null, Box_::class);
$bound($a); echo $a->secret()['b'], "\n";

// Readonly: modifying an element is an error, even after the cache warmed up.
function ro($o) { $o->ro['b'] = 2; }
try { ro($a); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// Not an array yet: null autovivifies; a string offset write; a scalar fails.
function setNA($o, $v) { $o->notArray['x'] = $v; }
setNA($a, 1); var_dump($a->notArray);
$a->notArray = "abc"; try { setNA($a, "z"); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$a->notArray = 5; try { setNA($a, 1); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($a->notArray);

// ArrayAccess in a property.
class AA implements ArrayAccess {
    public $log = [];
    function offsetExists($o): bool { return true; }
    function offsetGet($o): mixed { return null; }
    function offsetSet($o, $v): void { $this->log[] = "$o=$v"; }
    function offsetUnset($o): void {}
}
$a->items = new AA; setItem($a, 'q', 1); setItem($a, 'w', 2); echo implode(",", $a->items->log), "\n";

// Dynamic properties and __get/__set objects.
$o = new stdClass; $o->list = []; for ($i = 0; $i < 2000; $i++) setItem2($o, $i % 3); echo json_encode($o), "\n";
function setItem2($o, $k) { $o->list[$k] = $k * 2; }

// An error handler that throws is surfaced by the next operation.
set_error_handler(function ($no, $msg) { throw new RuntimeException($msg); });
for ($i = 0; $i < 3; $i++) {
    try { $u = []; $v = $u['missing']; echo "no\n"; } catch (RuntimeException $e) { echo "caught: ", $e->getMessage(), "\n"; }
}
restore_error_handler();
echo "done\n";
