<?php
class A {
    public static function make(): static { return new static(); }
    public function me(): self { return $this; }
    public function n(): ?int { return null; }
    public function u(): int|string { return 1.0; }
}
class B extends A {}
function f1(): float { return 3; }
function f2(): int { return "42"; }
function f3(): string { return 12; }
function f4(): void { return; }
function f5(): bool { return 1; }
function f6(): array { return "x"; }
function f7(): iterable { return [1]; }
function f8(): ?A { return new B; }
function f9(): never { throw new LogicException("never"); }
function f10($x): int { return $x; }
$out = [];
for ($i = 0; $i < 4; $i++) {
    $out[] = var_export([f1(), f2(), f3(), f4(), f5(), f7(), get_class(f8()), get_class(B::make()), get_class((new B)->me()), (new A)->n(), (new A)->u()], true);
    try { f6(); } catch (TypeError $e) { $out[] = $e->getMessage(); }
    try { f9(); } catch (LogicException $e) { $out[] = $e->getMessage(); }
    try { f10([]); } catch (TypeError $e) { $out[] = $e->getMessage() . " @" . $e->getLine(); }
    $out[] = f10("7") + f10(1.0);
}
echo implode("\n", $out), "\n";
