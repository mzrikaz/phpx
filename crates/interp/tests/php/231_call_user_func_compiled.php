<?php
namespace Shadow {
    function call_user_func($f, ...$a) { return 'shadowed'; }
    function run() { return call_user_func('strlen', 'abc'); }
}
namespace {
function g($x) { return "g($x)"; }
class A {
    private function p($x) { return "p($x)"; }
    function m($x) { return "m($x)"; }
    static function s($x) { return "s($x)"; }
    function viaSelf($x) { return call_user_func([$this, 'p'], $x); }
}
class B extends A { function viaB($x) { return call_user_func([$this, 'p'], $x); } }
function run($i, $o, $cl) {
    $out = [];
    $out[] = call_user_func('g', $i);
    $out[] = call_user_func('\g', $i);
    $out[] = call_user_func('strtoupper', 'x');
    $out[] = call_user_func($cl, $i);
    $out[] = call_user_func(fn($x) => $x * 2, $i);
    $out[] = call_user_func([$o, 'm'], $i);
    $out[] = call_user_func([$o, 's'], $i);
    $out[] = call_user_func('A::s', $i);
    $out[] = call_user_func(['A', 's'], $i);
    $out[] = $o->viaSelf($i);
    $out[] = call_user_func('max', 1, $i, 3);
    $out[] = \call_user_func('g', 'abs');
    foreach (['nofn', [$o, 'p'], [new B, 'p'], [$o, 'nope'], 42, [$o]] as $bad) {
        try { $out[] = call_user_func($bad, $i); } catch (Error $e) { $out[] = get_class($e) . ': ' . $e->getMessage(); }
    }
    try { $out[] = (new B)->viaB($i); } catch (Error $e) { $out[] = get_class($e) . ': ' . $e->getMessage(); }
    return $out;
}
$o = new A; $cl = function ($x) { return "cl($x)"; };
for ($i = 0; $i < 40; $i++) $r = run($i, $o, $cl);
echo implode("\n", $r), "\n";
echo Shadow\run(), "\n";
}
