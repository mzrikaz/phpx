<?php
class A {
    public static function create() { return new static(); }
    public static function who() { return static::class; }
    public static function viaSelf() { return self::who(); }
    public static function viaStatic() { return static::who(); }
    public function hello($x) { return "A::hello($x) on " . get_class($this); }
    private static function secret() { return "secret"; }
    public static function callSecret() { return self::secret(); }
    protected static function prot() { return "prot " . static::class; }
    public function typed(): int { return "12"; }
}
class B extends A {
    public function hello($x) { return "B+" . parent::hello($x); }
    public static function viaParent() { return parent::who(); }
    public static function callProt() { return parent::prot(); }
}
class C extends B {}
function loop($cls, $n) {
    $out = [];
    for ($i = 0; $i < $n; $i++) {
        $out[] = A::who() . "/" . $cls::who();
    }
    return implode(",", $out);
}
for ($i = 0; $i < 3; $i++) {
    echo A::who(), " ", B::who(), " ", C::who(), "\n";
    echo A::viaSelf(), " ", B::viaSelf(), " ", C::viaStatic(), " ", C::viaParent(), "\n";
    echo get_class(A::create()), " ", get_class(C::create()), "\n";
    echo (new B)->hello($i), " | ", (new C)->hello($i), "\n";
    echo A::callSecret(), " ", B::callProt(), " ", C::callProt(), "\n";
    echo loop('B', 2), " ", loop('C', 2), "\n";
}
function tryit($f) { try { echo $f(), "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
for ($i = 0; $i < 3; $i++) {
    tryit(fn() => A::secret());
    tryit(fn() => A::prot());
    tryit(fn() => A::hello(1));
    tryit(fn() => A::nope());
    tryit(fn() => Nope::x());
    tryit(fn() => var_export((new A)->typed(), true));
}
