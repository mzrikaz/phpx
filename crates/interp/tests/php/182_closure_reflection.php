<?php
class M { function toBeTrue() { return 1; } static function s() {} }
$o = new M;
foreach ([Closure::fromCallable([$o, 'toBeTrue']), $o->toBeTrue(...), Closure::fromCallable('M::s'), function () {}, (function () { return function () {}; })->call($o), Closure::fromCallable('strlen')] as $i => $c) {
  $r = new ReflectionFunction($c);
  echo $i, ' ', var_export($r->getClosureThis() === $o, true), ' ', $r->getClosureScopeClass()?->getName() ?? 'null', ' ', preg_replace('/\{closure:.*\}/', '{closure}', $r->getName()), ' ', $r->getClosureCalledClass()?->getName() ?? 'null', "\n";
}
