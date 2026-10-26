<?php
class X {}
class Y extends X {}
class Untyped { public $p; }
class Typed { public int $p = 0; }
class Holder { public ?X $x = null; }
function w($o, $v) { $o->p = $v; }
function hx($o, $v) { $o->x = $v; }
function run() {
    $u = new Untyped; $t = new Typed; $h = new Holder;
    for ($i = 0; $i < 1200; $i++) { w($u, new X); w($t, 5); hx($h, new Y); hx($h, null); }
    try { w($t, new X); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
    var_dump($t->p, $t->p + 1);
    hx($h, new Y); echo get_class($h->x), "\n";
    try { hx($h, new stdClass); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
    try { hx($h, 5); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}
run(); run();
