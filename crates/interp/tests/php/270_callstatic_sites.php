<?php
// Static calls answered by __callStatic, from compiled call sites.
class Facade {
    public static function __callStatic($name, $args) { return static::class . "::$name(" . implode(',', $args) . ")"; }
}
class Route extends Facade { public static function real() { return 'real'; } }
class Cache extends Facade {}
class Both {
    public function __call($n, $a) { return "__call $n"; }
    public static function __callStatic($n, $a) { return "__callStatic $n"; }
    public function inside() { return [self::missing(1), static::missing(2), Both::missing(3)]; }
    public static function fromStatic() { return self::missing(4); }
}
class Thrower { public static function __callStatic($n, $a) { throw new LogicException("no $n"); } }
class Priv { private static function hidden() { return 'hidden'; } public static function __callStatic($n, $a) { return "magic $n"; } public static function inner() { return self::hidden(); } }
function run($i) {
    $r = [Route::get('/a', $i), Cache::remember('k', 60), Route::real(), Route::get(), Facade::any(...[1, 2])];
    $r[] = (new Both)->inside();
    $r[] = Both::fromStatic();
    try { Thrower::boom($i); } catch (LogicException $e) { $r[] = $e->getMessage() . " " . $e->getLine(); }
    $r[] = Priv::hidden(); $r[] = Priv::inner();
    $big = str_repeat('x', 3); $r[] = Route::with($big, [1, 2], null);
    return $r;
}
for ($i = 0; $i < 4; $i++) echo json_encode(run($i)), "\n";
