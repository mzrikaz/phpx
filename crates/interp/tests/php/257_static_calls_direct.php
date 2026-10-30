<?php
// `parent::m()` / `self::m()` / `A::m()` called directly from compiled
// code: instance methods get the caller's `$this` (of the class cached),
// static methods none; `static` return types checked inline.
class Base {
    public function __construct(protected string $tag) {}
    public function who(): string { return static::class . ':' . $this->tag; }
    public static function kind(): string { return isset($this) ? 'has this' : 'static:' . static::class; }
    public function copy(): static { return new static($this->tag); }
    public function wrong(): static { return new Base('x'); }
}
class A extends Base {
    public function __construct(private int $n) { parent::__construct('a' . $n); }
    public function who(): string { return 'A>' . parent::who() . '|' . self::kind() . '|' . Base::kind(); }
    public function copy(): static { return new static($this->n + 1); }
}
class B extends Base {
    public function __construct() { parent::__construct('b'); }
    public function who(): string { return 'B>' . parent::who() . '|' . parent::kind(); }
}
final class C extends B {
    public function who(): string { return 'C>' . parent::who(); }
}
$out = [];
for ($i = 0; $i < 1500; $i++) {
    $o = match ($i % 3) { 0 => new A($i), 1 => new B, 2 => new C };
    $w = $o->who();
    $c = $o->copy();
    if ($i % 500 < 3) $out[] = [$w, get_class($c), $c->who()];
    try { $o->wrong(); } catch (TypeError $e) { if ($i < 3) $out[] = $e->getMessage(); }
}
echo json_encode($out, JSON_PRETTY_PRINT), "\n";
echo Base::kind(), " ", A::kind(), "\n";
