<?php
// $obj->$name() through per-name call sites: case, visibility, __call, classes.
class A { function hello() { return 'A::hello'; } private function secret() { return 'A::secret'; }
  function viaSelf($o, $m) { return $o->$m(); } function __call($n, $a) { return "A::__call($n)"; } }
class B extends A { function hello() { return 'B::hello'; } }
class C { function hello() { return 'C::hello'; } }
function call($o, $m) { return $o->$m(); }
for ($i = 0; $i < 3; $i++) {
  foreach ([new A, new B, new C] as $o) { foreach (['hello', 'HELLO', 'Hello'] as $m) { echo call($o, $m), ' '; } echo "\n"; }
  echo call(new A, 'secret'), ' ', (new A)->viaSelf(new A, 'secret'), ' ', (new A)->viaSelf(new B, 'secret'), "\n";
  echo call(new B, 'missing'), "\n";
  try { call(new C, 'missing'); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
  try { $m = 'x'; null->$m(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
  $m = 5; try { echo (new A)->$m(), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
  echo call_user_func([new B, 'hello']), ' ', call_user_func([new A, 'HELLO']), ' ', "\n";
}
