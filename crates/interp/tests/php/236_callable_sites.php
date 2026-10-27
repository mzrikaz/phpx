<?php
namespace NS { function g($x) { return "ns:$x"; } class C { static function s($x) { return "C::s:$x"; } } }
namespace {
function g($a, $b = 'd') { return "g:$a,$b"; }
function r(&$x) { $x++; return $x; }
class A {
  public static function s(...$a) { return 'A::s:' . implode(',', $a) . ':' . static::class; }
  private static function p($x) { return "A::p:$x"; }
  public function m($x) { return "m:$x:" . get_class($this); }
  public function inst() { return call_user_func('A::m', 5) . '|' . call_user_func_array('A::p', [6]) . '|' . call_user_func_array([$this, 'p'], [7]); }
  public static function viaSelf() { return call_user_func('A::p', 8); }
}
class B extends A {}
$o = new B;
for ($i = 0; $i < 3; $i++) {
  echo call_user_func('g', $i), ' ', call_user_func_array('g', [$i, 'x']), ' ', call_user_func_array('g', [$i]), "\n";
  echo call_user_func('A::s', $i, 2), ' ', call_user_func('B::s', $i), ' ', call_user_func('\A::s'), ' ', call_user_func_array('b::S', []), "\n";
  echo call_user_func('NS\g', $i), ' ', call_user_func_array('\NS\C::s', [$i]), "\n";
  echo call_user_func_array([$o, 'm'], [$i]), ' ', call_user_func_array([$o, 'M'], ['x' => $i]), "\n";
  $args = [$i, 'y'];
  echo call_user_func_array('g', $args), ' ', call_user_func_array('g', ['b' => 'n', 'a' => $i]), "\n";
  $cb = ($i % 2) ? 'A::s' : 'g';
  echo call_user_func($cb, $i), "\n";
  $cl = fn($x) => $x * 2;
  echo call_user_func_array($cl, [$i]), ' ', call_user_func_array($cl, [3 => $i]), "\n";
}
echo $o->inst(), "\n";
echo A::viaSelf(), "\n";
$v = 1; $arr = [&$v];
echo call_user_func_array('r', $arr), " $v\n";
foreach (['A::p', 'A::nope', 'Nope::x', 'nofn', 'A::'] as $bad) {
  try { echo call_user_func($bad, 1), "\n"; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
  try { echo call_user_func_array($bad, [1]), "\n"; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
try { echo call_user_func('A::m', 1), "\n"; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
