<?php
class A { function m() {} static function s() {} private function p() {} function __invoke() {} }
class B { function __call($n, $a) {} static function __callStatic($n, $a) {} }
$o = new A;
foreach ([null, 'strlen', 'nope', 'A::s', 'A::m', 'A::zz', 'B::zz', 'Nope::x', [$o, 'm'], [$o, 'zz'], [new B, 'zz'], ['A', 's'], ['B', 'q'], ['A'], [1, 2], $o, new stdClass, function () {}, strlen(...), 42, 'parent::x'] as $v) echo json_encode(is_callable($v)), ' ';
echo "\n";
