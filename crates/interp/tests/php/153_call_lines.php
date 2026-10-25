<?php
function g() { return 1; }
function f($v) { echo debug_backtrace()[0]['line'], "\n"; }
class A { function m($x) { echo debug_backtrace()[0]['line'], "\n"; return $this; } static function s($x) { echo debug_backtrace()[0]['line'], "\n"; } }
f(
  g()
);
(new A)
  ->m(
     g()
  )
  ->m(1);
A::s(
  g()
);
try {
  $x = [
    'k' => new Exception(
       'm' . g()
    ),
  ];
  throw $x['k'];
} catch (Exception $e) { echo $e->getLine(), "\n"; }
