<?php
class E {}
class P1 { public $a = 1; }
class P5 { public $a = 1; public $b = 2; public $c = 3; public $d = 4; public $e = 5; }
class C1 { public $a; public function __construct($a) { $this->a = $a; } }
class C2 { public function __construct(public int $a, public string $b) {} }
function b1($N) { for ($i = 0; $i < $N; $i++) { $o = new E; } }
function b2($N) { for ($i = 0; $i < $N; $i++) { $o = new P1; } }
function b3($N) { for ($i = 0; $i < $N; $i++) { $o = new P5; } }
function b4($N) { for ($i = 0; $i < $N; $i++) { $o = new C1($i); } }
function b5($N) { for ($i = 0; $i < $N; $i++) { $o = new C2($i, "x"); } }
function b6($N) { $a = []; for ($i = 0; $i < $N; $i++) { $a[] = new P1; } return $a; }
$N = 3000000;
foreach (['b1'=>'new E','b2'=>'new P1','b3'=>'new P5','b4'=>'new C1($i)','b5'=>'new C2 promoted'] as $f => $d) {
  $t = microtime(true); $f($N); printf("%-16s %5.1f ns\n", $d, (microtime(true)-$t)*1e9/$N);
}
