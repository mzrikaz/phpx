<?php function f(int|string|null|bool|array|Foo|float $x) {} try { f(new stdClass); } catch (TypeError $e) { echo preg_replace("/, called.*/", "", $e->getMessage()), "\n"; } echo (new ReflectionFunction("f"))->getParameters()[0]->getType(), "\n";

interface A {} interface B {}
function g((A&B)|null $x, INT $y, ?Foo $z, iterable|string $w) {}
foreach ([[new stdClass, 1, null, ''], [null, 'x', null, ''], [null, 1, 5, ''], [null, 1, null, 1.5]] as $args) { try { g(...$args); } catch (TypeError $e) { echo preg_replace('/, called.*/', '', $e->getMessage()), "\n"; } }
