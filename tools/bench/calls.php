<?php
function e() {}
function one($a) { return $a; }
function ints($a) { return $a + 1; }
class P { public $p = 1; public function get() { return $this->p; } public function e() {} public static function s() {} }
function b1($N) { for ($i = 0; $i < $N; $i++) { e(); } }
function b2($N) { for ($i = 0; $i < $N; $i++) { one($i); } }
function b3($N) { $o = new P; for ($i = 0; $i < $N; $i++) { $o->e(); } }
function b4($N) { $o = new P; for ($i = 0; $i < $N; $i++) { $o->get(); } }
function b5($N) { for ($i = 0; $i < $N; $i++) { P::s(); } }
function b6($N) { for ($i = 0; $i < $N; $i++) { $x = one("s"); } }
$N = 10000000;
foreach (['b1'=>'e()','b2'=>'one($i)','b3'=>'$o->e()','b4'=>'$o->get()','b5'=>'P::s()','b6'=>'$x=one("s")'] as $f => $d) {
  $t = microtime(true); $f($N); printf("%-14s %5.1f ns\n", $d, (microtime(true)-$t)*1e9/$N);
}
