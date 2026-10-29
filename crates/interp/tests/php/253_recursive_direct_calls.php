<?php
// Compiled calls reuse a frame per call depth at each call site:
// recursion, calls suspended in fibers (frames released out of order),
// the cached callee changing mid-recursion, and exceptions unwinding.
function depth(int $n, int $acc): int { return $n === 0 ? $acc : depth($n - 1, $acc + $n); }
function fib(int $n): int { return $n < 2 ? $n : fib($n - 1) + fib($n - 2); }

abstract class Expr { abstract public function ev(): int; }
final class Num extends Expr { public function __construct(private int $v) {} public function ev(): int { return $this->v; } }
final class Add extends Expr { public function __construct(private Expr $a, private Expr $b) {} public function ev(): int { return $this->a->ev() + $this->b->ev(); } }
final class Mul extends Expr { public function __construct(private Expr $a, private Expr $b) {} public function ev(): int { return $this->a->ev() * $this->b->ev(); } }
function tree(int $d): Expr { return $d === 0 ? new Num($d + 2) : ($d % 2 ? new Add(tree($d - 1), tree($d - 2 < 0 ? 0 : $d - 2)) : new Mul(tree($d - 1), new Num(1))); }

// Suspends from deep inside a recursive call; another fiber recurses
// through the same call sites meanwhile.
function walk(int $n, string $who, array &$log): int {
    if ($n === 0) {
        $log[] = "$who bottom";
        Fiber::suspend($who);
        $log[] = "$who resumed";
        return 0;
    }
    $r = walk($n - 1, $who, $log) + $n;
    $log[] = "$who up $n";
    return $r;
}

function thrower(int $n): int { if ($n === 0) throw new RuntimeException("bottom"); return thrower($n - 1) + 1; }

$sum = 0;
for ($i = 0; $i < 2000; $i++) { $sum += depth($i % 50, 0) + fib($i % 12) + tree($i % 7)->ev(); }
echo $sum, "\n";

$log = [];
for ($round = 0; $round < 3; $round++) {
    $f1 = new Fiber(function () use (&$log) { return walk(5, 'a', $log); });
    $f2 = new Fiber(function () use (&$log) { return walk(3, 'b', $log); });
    $f1->start(); $f2->start();
    $mid = depth(30, 0) + fib(10);
    $f1->resume(); $f2->resume();
    echo $f1->getReturn(), " ", $f2->getReturn(), "\n";
}
echo implode(",", array_slice($log, 0, 14)), "\n";

for ($i = 0; $i < 300; $i++) {
    try { thrower($i % 20); } catch (RuntimeException $e) { $sum++; }
}
echo $sum, " ", depth(40, 1), " ", fib(15), "\n";
