<?php
class A {
    public static $foo = 1;
    public static $deps = [];
    static function m() { return "m called"; }
    static function run($n) {
        self::$$n = 10;
        self::${$n . ''} += 5;
        echo self::$$n, "\n";
        self::${'deps'}['x'] = 'y';
        var_dump(isset(self::$$n), isset(self::${'nope'}));
        $r = &static::$$n; $r = 42;
        echo A::$foo, "\n";
        $p = 'deps'; $q = 'p';
        print_r(self::$$$q);
        $mm = 'm'; $k = 'mm';
        echo self::$$k(), "\n";
    }
}
$n = 'foo';
A::run($n);
$c = 'A';
echo $c::$$n, "\n";
try { echo A::${'missing'}; } catch (Error $e) { echo $e->getMessage(), "\n"; }
