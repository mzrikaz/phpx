<?php
class P { public $u = 1; public int $i = 0; public ?string $s = null; private float $f = 1.5; public readonly int $ro;
  function __construct() { $this->ro = 3; }
  function u() { return $this->u; } function i(): int { return $this->i; } function s(): ?string { return $this->s; }
  function f(): float { return $this->f; } function fi(): int { return $this->f; } function k(): string { return "k"; } function ro(): int { return $this->ro; }
  function setU($v) { $this->u = $v; } function setI(int $v): void { $this->i = $v; } function setS(?string $s): static { $this->s = $s; return $this; }
  function setF(float $f): self { $this->f = $f; return $this; } }
class Q extends P { function u() { return "Q"; } }
function run($objs) { $r = [];
  foreach ($objs as $o) for ($n = 0; $n < 3; $n++) {
    $o->setU([$n]); $o->setI($n); $o->setS($n ? "s$n" : null)->setF($n + 0.5);
    $r[] = [$o->u(), $o->i(), $o->s(), $o->f(), $o->k(), $o->ro()];
    try { $o->setI("7"); $r[] = $o->i(); } catch (TypeError $e) { $r[] = $e->getMessage(); }
    try { $o->setF(2); $r[] = $o->f(); } catch (TypeError $e) { $r[] = $e->getMessage(); }
    try { $r[] = $o->fi(); } catch (TypeError $e) { $r[] = $e->getMessage(); }
  }
  return $r; }
for ($k = 0; $k < 3; $k++) $out = run([new P, new Q, new P]);
echo json_encode($out), "\n";
class D { function __destruct() { echo "D gone\n"; } }
$p = new P; for ($k = 0; $k < 3; $k++) { $p->setU(new D); echo "set $k\n"; }
echo "after\n";

// Uninitialized typed properties, private setters, readonly, unset slots.
class R { public int $n; public $m = 1; public readonly int $ro;
  function n(): int { return $this->n; } function setN(int $n) { $this->n = $n; }
  function m() { return $this->m; } function setRo(int $v) { $this->ro = $v; }
  private function hide($v) { $this->m = $v; } function useHide($v) { $this->hide($v); return $this->m; }
  function arr() { return $this->m; } }
function probe($objs) {
  foreach ($objs as $i => $o) {
    try { echo $o->n(), "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    $o->setN($i); echo $o->n(), "\n";
    try { $o->setRo($i); echo $o->ro, "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { $o->hide(1); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    echo $o->useHide($i * 2), "\n";
    if ($i == 1) unset($o->m);
    try { var_dump($o->m()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    $o->setN(5);
  }
}
for ($k = 0; $k < 3; $k++) probe([new R, new R, new R]);

// The returned array is a copy; the setter's old value is released.
class A { public $a = [1, 2]; function a() { return $this->a; } function setA($a) { $this->a = $a; } }
function arrays() { $o = new A; for ($i = 0; $i < 3; $i++) { $x = $o->a(); $x[] = $i; $o->setA($x); } return $o->a(); }
for ($k = 0; $k < 3; $k++) echo json_encode(arrays()), "\n";

// One call site, several classes (an interface's): each answered as its own.
interface Named { function name(): string; }
class N1 implements Named { function __construct(private string $n) {} function name(): string { return $this->n; } }
class N2 implements Named { protected string $label = "two"; function name(): string { return $this->label; } }
class N3 implements Named { function name(): string { return "three"; } }
class N4 implements Named { public $v = 4; function name(): string { return $this->v; } }
function names(array $xs) { $out = []; foreach ($xs as $x) { $out[] = $x->name(); $out[] = $x->name() === 'two' ? 'is two' : 'not two'; $out[] = $x->name() !== 'one'; } return $out; }
for ($k = 0; $k < 3; $k++) echo json_encode(names([new N1("one"), new N2, new N3, new N4, new N1("uno")])), "\n";
