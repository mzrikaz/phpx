<?php
namespace NS {
    function g($x) { return "NS\\g($x)"; }
}
namespace {
function g($x) { return "g($x)"; }
function inc(&$x) { $x++; return $x; }
class A {
    private function p() { return 'p'; }
    function m($x) { return "m($x)"; }
    static function s($x) { return "s($x)"; }
    function __call($n, $a) { return "__call($n)"; }
    function callPriv() { $c = [$this, 'p']; return $c(); }
}
class B { private function p() { return 'p'; } function m() { return 'Bm'; } }
function run($i) {
    $out = [];
    $f = 'g'; $out[] = $f($i);
    $f = '\g'; $out[] = $f($i);
    $f = 'NS\g'; $out[] = $f($i);
    $f = 'G'; $out[] = $f($i);
    $f = 'inc'; $n = $i; $f($n); $out[] = $n;
    $f = 'sort'; $arr = [3, 1, 2]; $f($arr); $out[] = implode(',', $arr);
    $f = 'strtoupper'; $out[] = $f('x');
    $o = new A;
    $c = [$o, 'm']; $out[] = $c($i);
    $c = [$o, 's']; $out[] = $c($i);
    $c = [$o, 'nope']; $out[] = $c($i);
    $c = [$o, 'p']; $out[] = $c($i);
    $out[] = $o->callPriv();
    $c = [new B, 'M']; $out[] = $c();
    foreach (['nofn', [new B, 'p'], [new B, 'nope'], [new B, 'A::m']] as $bad) {
        try { $out[] = $bad(); } catch (Error $e) { $out[] = get_class($e) . ': ' . $e->getMessage(); }
    }
    return $out;
}
for ($i = 0; $i < 30; $i++) $r = run($i);
echo implode("\n", $r), "\n";
function later_target() { return 'later'; }
$f = 'later_fn';
$g = 'preg_match'; $g('/(b)/', 'abc', $m); echo json_encode($m), "\n";
try { echo $f(), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
function later_fn() { return 'defined'; }
}
