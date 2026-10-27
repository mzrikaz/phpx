<?php
// Element stores the compiled code shortcuts: copies stay unchanged,
// references are written through, numeric-string keys become ints,
// replaced values are released, and the general cases still apply.
class D { function __construct(public $n) {} function __destruct() { echo "drop {$this->n}\n"; } }
class Box {
    public $arr = [];
    public array $typed = [1, 2];
    private $hidden = [];
    public readonly array $ro;
    function __construct() { $this->ro = [1]; }
    function setHidden($k, $v) { for ($i = 0; $i < 3; $i++) $this->hidden[$k] = $v . $i; return $this->hidden; }
}
class AA implements ArrayAccess {
    public $log = [];
    function offsetExists($o): bool { return true; }
    function offsetGet($o): mixed { return null; }
    function offsetSet($o, $v): void { $this->log[] = "$o=$v"; }
    function offsetUnset($o): void {}
}
function locals() {
    $a = [0, 1, 2];
    $b = $a;
    for ($i = 0; $i < 3; $i++) $a[$i] = $i * 10;
    echo json_encode([$a, $b]), "\n";
    $h = ['x' => 1];
    $c = $h;
    for ($i = 0; $i < 3; $i++) { $h['x'] = $i; $h['y'] = -$i; $h['5'] = 'five'; $h[-3] = 'neg'; }
    echo json_encode([$h, $c]), "\n";
    var_dump(array_keys($h));
    $r = [1, 2];
    $x = &$r[1];
    for ($i = 0; $i < 2; $i++) $r[1] = 'ref' . $i;
    echo $x, "\n";
    $d = [new D(1), 'k' => new D(2)];
    $d[0] = 'plain';
    $d['k'] = 3;
    echo "after\n";
    $s = 'abc';
    $s[1] = 'X';
    echo $s, "\n";
    $n = null;
    $n['a'] = 1;
    echo json_encode($n), "\n";
    $f = [2.0 => 'a'];
    $f[true] = 'b';
    echo json_encode($f), "\n";
}
function props() {
    $o = new Box;
    $copy = $o->arr;
    for ($i = 0; $i < 3; $i++) { $o->arr['k'] = $i; $o->arr[0] = $i; $o->arr['7'] = 'seven'; }
    echo json_encode([$o->arr, $copy]), "\n";
    for ($i = 0; $i < 3; $i++) $o->typed[1] = $i + 5;
    echo json_encode($o->typed), "\n";
    print_r($o->setHidden('h', 'v'));
    try { $o->ro[0] = 2; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { $o->hidden['x'] = 1; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    $w = new stdClass;
    $w->list = new AA;
    for ($i = 0; $i < 2; $i++) $w->list['a'] = $i;
    echo implode(',', $w->list->log), "\n";
    $p = new Box;
    $p->arr = [new D(3)];
    $p->arr[0] = 'gone';
    echo "props done\n";
}
locals();
props();
