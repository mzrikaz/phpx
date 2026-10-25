<?php
class D { function __construct(public $n) {} function __destruct() { echo "~{$this->n}\n"; } }
function f() { $a = new D("local"); echo "in f\n"; }
f(); echo "after f\n";
$g = new D("global");
$x = new D("x"); $x = null; echo "x nulled\n";
$arr = [new D("a1"), new D("a2")]; unset($arr[0]); echo "unset a1\n";
class T { function __destruct() { throw new Exception("boom"); } }
try { $t = new T; unset($t); echo "next\n"; } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
echo "end\n";
