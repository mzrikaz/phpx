<?php
declare(strict_types=0);
class Pt { public function __construct(public int $x = 0, public int $y = 0, protected ?string $tag = null) {} public function tag() { return $this->tag; } }
class Ro { public function __construct(public readonly string $name) {} }
class ByRef { public function __construct(&$out) { $out = "set by ctor"; } }
class Single { private static $inst; private function __construct() { echo "made\n"; } public static function get() { return self::$inst ??= new self(); } }
class Base { public $kind; public function __construct($k = 'base') { $this->kind = $k; } public static function make() { return new static('made'); } }
class Child extends Base { public function __construct() { parent::__construct('child'); } }
class Grand extends Child {}
class Boom { public function __construct($n) { if ($n % 2) throw new RuntimeException("odd $n"); } }
class D { public function __construct(public $n) {} public function __destruct() { echo "bye {$this->n}\n"; } }
for ($i = 0; $i < 3; $i++) {
    $p = new Pt($i, "5", $i ? "t$i" : null);
    var_dump($p->x, $p->y, $p->tag());
    $q = new Pt();
    echo $q->x + $q->y, "\n";
    $r = new Ro("n$i");
    echo $r->name, "\n";
    try { $r->name = 'x'; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    new ByRef($o); echo $o, "\n";
    Single::get(); Single::get();
    echo (new Base)->kind, " ", (new Child)->kind, " ", (new Grand)->kind, " ", Base::make()->kind, " ", get_class(Grand::make()), "\n";
    foreach ([1, 2] as $n) { try { new Boom($n + $i); echo "ok\n"; } catch (RuntimeException $e) { echo $e->getMessage(), "\n"; } }
    try { new Pt("abc"); } catch (TypeError $e) { echo str_replace(__FILE__, 'FILE', $e->getMessage()), " @", $e->getLine(), "\n"; }
    try { new Single(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { new Ro(); } catch (ArgumentCountError $e) { echo get_class($e), "\n"; }
    $d = new D($i); $d = null;
    $cls = $i % 2 ? 'Child' : 'Base';
    echo (new $cls)->kind, "\n";
}
echo "end\n";
