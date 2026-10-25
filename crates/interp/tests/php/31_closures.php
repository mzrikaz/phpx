<?php
namespace App;
class Counter {
    private $n = 0;
    public function inc() { return function ($by = 1) { $this->n += $by; return $this->n; }; }
    public static function make() { return static fn($x) => $x * 2; }
    public function named() { return fn() => __FUNCTION__ . "|" . __METHOD__ . "|" . __CLASS__; }
}
function outer() { return function () { return __FUNCTION__; }; }
$c = new Counter;
$inc = $c->inc();
for ($i = 0; $i < 3; $i++) { echo $inc(), $inc(2), " "; }
echo "\n";
$dbl = Counter::make();
echo $dbl(21), " ", (new Counter)->named()(), " ", outer()(), "\n";
// by-value capture is a snapshot, each call starts fresh
$x = 1;
$f = function () use ($x) { $x++; return $x; };
$x = 100;
echo $f(), $f(), $f(), "\n";
// by-reference capture shares
$total = 0;
$add = function ($n) use (&$total) { $total += $n; return $total; };
for ($i = 1; $i <= 4; $i++) { $add($i); }
echo $total, "\n";
// recursion through a by-ref captured closure
$fib = function ($n) use (&$fib) { return $n < 2 ? $n : $fib($n - 1) + $fib($n - 2); };
echo $fib(15), "\n";
// arrow fn captures by value at creation
$k = 5;
$af = fn($v) => $v + $k;
$k = 50;
echo $af(1), $af(2), "\n";
// callbacks from built-ins
echo implode(",", array_map(fn($v) => $v * $k, [1, 2, 3])), "\n";
$arr = [3, 1, 2]; usort($arr, fn($a, $b) => $b <=> $a); echo implode(",", $arr), "\n";
echo implode(",", array_filter([1, 2, 3, 4, 5, 6], function ($v) use ($k) { return $v % 2 == 0 && $v < $k; })), "\n";
echo array_reduce([1, 2, 3], fn($c, $v) => $c + $v, 10), "\n";
// type errors in closures
$typed = function (int $n): string { return "n=$n"; };
echo $typed("7"), "\n";
try { $typed("x"); } catch (\TypeError $e) { echo str_replace(__FILE__, 'FILE', $e->getMessage()), " @", $e->getLine(), "\n"; }
try { array_map(fn(int $z) => $z, ["q"]); } catch (\TypeError $e) { echo str_replace(__FILE__, 'FILE', $e->getMessage()), " @", $e->getLine(), "\n"; }
$top = function () { return __FUNCTION__; };
echo str_replace(__FILE__, 'FILE', $top()), "\n";
// Closure::bind / call / fromCallable
class Secret { private $v = 'hidden'; }
$peek = function () { return $this->v; };
echo \Closure::bind($peek, new Secret, Secret::class)(), " ", $peek->call(new Secret), "\n";
$sl = \Closure::fromCallable('strtoupper'); echo $sl("abc"), " ", (strrev(...))("abc"), "\n";
// generator closure
$gen = function ($n) { for ($i = 0; $i < $n; $i++) yield $i => $i * $i; };
foreach ($gen(4) as $kk => $vv) echo "$kk:$vv ";
echo "\n";
// many closures created in a loop, captured loop var
$fs = [];
for ($i = 0; $i < 5; $i++) { $fs[] = function () use ($i) { return $i * 10; }; }
echo implode(",", array_map(fn($f) => $f(), $fs)), "\n";
// closure stored in property and called
class Holder { public $cb; public function __construct() { $this->cb = fn($a) => strtoupper($a); } public function run($s) { return ($this->cb)($s); } }
echo (new Holder)->run("done"), "\n";
