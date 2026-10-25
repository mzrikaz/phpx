<?php
trait T { protected array $attrs; function get() { return $this->attrs ??= ['x']; } }
class A { use T; function __get($p) { echo "__get $p\n"; return null; } function __isset($p) { echo "__isset $p\n"; return false; } }
$a = new A; var_dump($a->get());
class B { protected array $attrs; function __get($p) { echo "__get $p\n"; return null; } function __isset($p) { echo "__isset $p\n"; return false; }
  function t() { var_dump(isset($this->attrs)); var_dump($this->attrs ?? 'd'); $this->attrs ??= [1]; var_dump($this->attrs); } }
(new B)->t();
class D { public int $n; function __get($p) { echo "__get $p\n"; return 5; } }
$d = new D; var_dump(isset($d->n), $d->n ?? 'def');
try { var_dump($d->n); } catch (Error $e) { echo $e->getMessage(), "\n"; }
unset($d->n); var_dump($d->n);
class M { public array $arr; public ?int $n; public $plain;
  function __get($p) { echo "__get $p\n"; return 7; }
  function __set($p, $v) { echo "__set $p\n"; }
  function __isset($p) { echo "__isset $p\n"; return true; }
  function __unset($p) { echo "__unset $p\n"; } }
$m = new M;
$m->arr[] = 1; var_dump($m->arr);
$m2 = new M; unset($m2->n); var_dump(isset($m2->n)); var_dump($m2->n); $m2->n = 3; var_dump($m2->n);
unset($m2->n); $m2->n = 4; 
$m3 = new M; $m3->n = 1; unset($m3->n); var_dump($m3->n);
unset($m3->plain); var_dump($m3->plain); $m3->plain = 1;
$m4 = new M; var_dump(isset($m4->n)); var_dump(property_exists($m4, 'n')); var_dump((array)$m4); 
$c = clone $m2; var_dump($c->n);
class G { private $data = [];
  function __get($n) { echo "get $n\n"; return $this->$n ?? 'dflt'; }
  function __set($n, $v) { echo "set $n\n"; $this->$n = $v; }
  function __isset($n) { echo "isset $n\n"; return isset($this->$n); }
  function __unset($n) { echo "unset $n\n"; unset($this->$n); } }
$g = new G; var_dump($g->x); $g->y = 5; var_dump($g->y); var_dump(isset($g->z)); unset($g->w);
var_dump($g->data);
class L { public $lazy; function __construct() { unset($this->lazy); }
  function __get($n) { echo "load $n\n"; $this->$n = 42; return $this->$n; } }
$l = new L; var_dump($l->lazy, $l->lazy);
