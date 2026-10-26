<?php
namespace App;
interface I {}
class A implements I { const K = 'ka'; public static $sp = ['x' => 1]; public $v;
  function __construct($v = 0) { $this->v = $v; }
  static function make($v) { return new static($v); } static function name() { return static::class; }
  function inst() { return $this::K . '/' . $this::name() . '/' . $this::class . '/' . get_class(new $this(5)); }
  function stat() { $c = static::class; $c::$sp['y'] = 2; $c::$sp['x']++; return $c::$sp; } }
class B extends A { const K = 'kb'; }
function run($cls, $obj) {
  $o = new $cls(3); echo get_class($o), ' ', $o->v, "\n";
  echo $cls::K, ' ', $obj::K, ' ', $cls::name(), ' ', $obj::name(), ' ', $obj::class, "\n";
  var_dump($obj instanceof $cls, $obj instanceof $obj, $o instanceof I, $obj instanceof \stdClass);
  $n = 'App\I'; var_dump($obj instanceof $n); $nn = 'Nope\Missing'; var_dump($obj instanceof $nn);
  echo $obj->inst(), "\n"; var_dump($obj->stat());
  $m = $cls::make(7); echo get_class($m), $m->v, "\n";
  try { $x = 'stdClass'; echo $x::class; } catch (\TypeError $e) { echo $e->getMessage(), "\n"; }
  try { $x = null; echo $x::class; } catch (\TypeError $e) { echo $e->getMessage(), "\n"; }
  try { $x = 5; $x::foo(); } catch (\Error $e) { echo $e->getMessage(), "\n"; }
  try { $x = 'Nope\Missing'; new $x; } catch (\Error $e) { echo $e->getMessage(), "\n"; }
  try { $x = []; var_dump($obj instanceof $x); } catch (\Error $e) { echo $e->getMessage(), "\n"; }
}
for ($i = 0; $i < 3; $i++) { run('App\A', new B); run(B::class, new A); }
