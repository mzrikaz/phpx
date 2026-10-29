<?php
// Declared return types checked inline in compiled code: scalars of the
// declared type, null for nullable types, objects of classes accepted
// before; everything else is coerced or rejected as in PHP.
interface Animal { public function name(): string; }
class Dog implements Animal { public function name(): string { return 'dog'; } public static function make(): static { return new static; } public function me(): self { return $this; } }
class Puppy extends Dog {}
class Cat implements Animal { public function name(): string { return 'cat'; } }
class Rock {}

function pick(int $i): Animal { return $i % 3 === 0 ? new Dog : ($i % 3 === 1 ? new Cat : new Puppy); }
function maybe(int $i): ?Dog { return $i % 2 ? new Dog : null; }
function num($v): int { return $v; }
function flt($v): float { return $v; }
function str($v): string { return $v; }
function opt_int($v): ?int { return $v; }
function either($v): int|string { return $v; }
function nothing(): void { return; }
function bad(int $i): Animal { return $i > 2000 ? new Rock : new Dog; }

$out = [];
for ($i = 0; $i < 2100; $i++) {
    $a = pick($i)->name();
    $m = maybe($i);
    $n = num($i) + flt($i) + strlen(str("x$i"));
    $o = opt_int($i % 2 ? $i : null);
    $s = Dog::make()::class . Puppy::make()::class . (new Puppy)->me()::class;
    if ($i % 700 === 3) $out[] = [$a, $m === null, $n, $o, $s, either($i), nothing()];
    try { bad($i); } catch (TypeError $e) { if ($i === 2001) $out[] = $e->getMessage(); }
}
echo json_encode($out), "\n";
var_dump(num("12"), num(3.0), num(true), flt(2), flt("1.5"), str(15), str(1.5), opt_int(null), either(2.0));
foreach ([fn() => num("x"), fn() => num(null), fn() => str([]), fn() => maybe2()] as $f) {
    try { var_dump($f()); } catch (TypeError $e) { echo $e->getMessage(), "\n"; } catch (Error $e) { echo get_class($e), "\n"; }
}
