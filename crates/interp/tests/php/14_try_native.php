<?php
namespace App;
class MyEx extends \RuntimeException {}
class D { function __construct(public $n) {} function __destruct() { echo "D{$this->n} "; } }
function thrower($x) { if ($x > 1) throw new MyEx("big $x"); return $x; }
function a($x) {
  try { $r = thrower($x) + 1; echo "ok $r\n"; }
  catch (MyEx $e) { echo "caught ", $e->getMessage(), "\n"; }
  finally { echo "fin\n"; }
  return "end";
}
function b($x) {
  try { return thrower($x); }
  catch (\LogicException $e) { echo "never\n"; }
  finally { echo "fin-b $x\n"; }
}
function c() {
  for ($i = 0; $i < 4; $i++) {
    try {
      if ($i == 1) continue;
      if ($i == 3) break;
      echo "i=$i\n";
    } finally { echo "f$i\n"; }
  }
  return $i;
}
function d() {
  try {
    try { throw new \InvalidArgumentException("inner"); }
    finally { echo "inner fin\n"; }
  } catch (\LogicException | MyEx $e) { echo get_class($e), " ", $e->getMessage(), "\n"; }
  try { throw new MyEx("x"); } catch (\Exception) { echo "no var\n"; }
  try { try { throw new MyEx("re"); } catch (MyEx $e) { throw new \LogicException("wrapped", 0, $e); } }
  catch (\Exception $e) { echo $e->getMessage(), "<-", $e->getPrevious()->getMessage(), "\n"; }
}
function e($arr) {
  $out = [];
  foreach ($arr as $v) {
    try { $out[] = "v" . thrower($v) . (new D($v))->n; }
    catch (MyEx $e) { $out[] = "E"; }
  }
  return implode(",", $out);
}
function f() {
  $x = "keep";
  try { return $x; } finally { echo "x still $x\n"; }
}
function g() {
  try { throw new MyEx("from g"); } finally { echo "g fin\n"; }
}
function h() {
  while (true) { try { try { return 5; } finally { echo "h1 "; } } finally { echo "h2\n"; } }
}
for ($k = 0; $k < 2; $k++) {
  echo a(1), "\n", a(2), "\n";
  echo b(1), "\n";
  try { b(5); } catch (\Exception $e) { echo "outer ", $e->getMessage(), "\n"; }
  echo c(), "\n";
  d();
  echo e([1, 2, 1, 3]), "\n";
  echo f(), "\n";
  try { g(); } catch (MyEx $e) { echo "got ", $e->getMessage(), " line ", $e->getLine(), "\n"; }
  echo h(), "\n";
}
try { throw new \Error("top"); } catch (\Error $e) { echo "top caught\n"; } finally { echo "top fin\n"; }
function z() { try { exit(3); } finally { echo "NO\n"; } }
z();
