<?php
class A { private function p() { return 'p'; } protected static function ps() { return 'ps'; } function m() { return 'm'; }
  function viaSelf() { return call_user_func([$this, 'p']) . call_user_func('A::ps') . call_user_func([$this, 'ps']) . array_map([$this, 'p'], [1])[0]; } }
class B extends A { function viaB() { try { return call_user_func([$this, 'p']); } catch (Throwable $e) { return get_class($e) . ': ' . $e->getMessage(); } } }
class M { private function p() { return 'p'; } function __call($n, $a) { return "__call $n"; } static function __callStatic($n, $a) { return "__callStatic $n"; } }
$cases = [
  fn() => call_user_func([new A, 'p']),
  fn() => call_user_func(['A', 'ps']),
  fn() => call_user_func('A::ps'),
  fn() => call_user_func([new A, 'nope']),
  fn() => call_user_func(['Nope', 'x']),
  fn() => call_user_func([1, 'x']),
  fn() => call_user_func([new A, 1]),
  fn() => call_user_func([new A]),
  fn() => call_user_func('A::m'),
  fn() => array_map([new A, 'p'], [1]),
  fn() => usort($x, [new A, 'p']),
  fn() => [new A, 'p'](),
  fn() => [new A, 'nope'](),
  fn() => ['A', 'ps'](),
  fn() => (new A)->viaSelf(),
  fn() => (new B)->viaB(),
  fn() => call_user_func([new M, 'p']),
  fn() => call_user_func(['M', 'p']),
  fn() => [new M, 'p'](),
  fn() => is_callable([new A, 'p']) ? 'y' : 'n',
  fn() => is_callable([new M, 'p']) ? 'y' : 'n',
];
foreach ($cases as $i => $c) {
  try { $r = $c(); echo $i, ': ', is_array($r) ? json_encode($r) : var_export($r, true), "\n"; } catch (Throwable $e) { echo $i, ': ', get_class($e), ': ', $e->getMessage(), "\n"; }
}
// Inside a closure bound to A, A's private methods are callable.
$f = Closure::bind(fn() => call_user_func([new A, 'p']) . is_callable([new A, 'p']), null, A::class);
echo $f(), "\n";
function cb(callable $c) { return $c(); }
try { cb([new A, 'p']); } catch (TypeError $e) { echo preg_replace('/, called in.*/', '', $e->getMessage()), "\n"; }
echo cb([new A, 'm']), "\n";
