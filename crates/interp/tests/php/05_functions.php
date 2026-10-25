<?php
function greet(string $greeting, string $name = "World", string ...$extra): string {
    return "$greeting, $name!" . ($extra ? " +" . implode(",", $extra) : "");
}
echo greet("Hi"), "\n", greet("Hi", "Bob"), "\n", greet("Hi", "Bob", "x", "y"), "\n", greet(name: "Ann", greeting: "Yo"), "\n";
echo greet(...["Hey", "Zed"]), "\n", greet(...['greeting' => 'Hola', 'name' => 'Ana']), "\n";
function sum(...$nums) { return array_sum($nums); }
echo sum(1, 2, 3), " ", sum(...[4, 5]), " ", sum(), "\n";
// Closures: by-value capture, by-ref capture, arrow fn auto-capture.
$factor = 3;
$mul = function ($x) use ($factor) { return $x * $factor; };
$factor = 100;
echo $mul(2), "\n";
$total = 0;
$add = function ($x) use (&$total) { $total += $x; };
$add(5); $add(7);
echo $total, "\n";
$base = 10;
$arrow = fn($x) => $x + $base;
$base = 20;
echo $arrow(1), "\n";
$counter = function () { static $c = 0; return ++$c; };
$counter(); echo $counter(), "\n";
$compose = fn(callable ...$fs) => fn($x) => array_reduce($fs, fn($acc, $f) => $f($acc), $x);
echo $compose(fn($x) => $x + 1, fn($x) => $x * 10, 'strval', strrev(...))(4), "\n";
// First-class callables and callable strings/arrays.
$len = strlen(...);
class MathOps { public static function sq($x) { return $x * $x; } public function cube($x) { return $x ** 3; } }
var_dump($len("four"), array_map('MathOps::sq', [1, 2]), array_map([new MathOps, 'cube'], [2]), array_map([MathOps::class, 'sq'], [3]), call_user_func('str_repeat', 'ab', 2), call_user_func_array([new MathOps, 'cube'], [3]));
$m = (new MathOps)->cube(...);
echo $m(2), " ", is_callable('strlen'), " ", is_callable('nope') ? 'y' : 'n', "\n";
// Recursion & static return.
function fact(int $n): int { return $n <= 1 ? 1 : $n * fact($n - 1); }
echo fact(20), "\n";
$fib = function (int $n) use (&$fib) { return $n < 2 ? $n : $fib($n - 1) + $fib($n - 2); };
echo $fib(15), "\n";
// Type coercion (non-strict file).
function takesInt(int $x) { var_dump($x); }
function takesFloat(float $x) { var_dump($x); }
function takesString(string $x) { var_dump($x); }
function takesBool(bool $x) { var_dump($x); }
function nullable(?int $x = null) { var_dump($x); }
function union(int|string $x) { var_dump($x); }
takesInt("42"); takesInt(3.0); takesInt(true); takesFloat(5); takesFloat("1.5"); takesString(12); takesString(1.5); takesBool(1); takesBool("");
nullable(); nullable(null); union(5); union("x"); union(5.0);
try { takesInt("abc"); } catch (TypeError $e) { echo get_class($e), "\n"; }
function ret(): float { return 1; }
var_dump(ret());
function compactTest() { $a = 1; $b = [2]; return compact('a', 'b'); }
var_dump(compactTest());
function args() { return func_get_args(); }
var_dump(args(1, 'two'));
// Nullsafe and match.
class U { public ?U $next = null; public function name() { return "u"; } }
$u = new U;
var_dump($u->next?->name(), $u?->name(), $u->next?->next->name());
$v = 7;
echo match (true) { $v < 5 => "small", $v < 10 => "medium", default => "large" }, "\n";
echo match ($v) { 1, 2, 3 => "low", 7, 8 => "seven-ish", default => "?" }, "\n";
try { echo match ($v) { 1 => 'x' }; } catch (\UnhandledMatchError $e) { echo $e->getMessage(), "\n"; }
try { echo match ("str") { 1 => 'x' }; } catch (\UnhandledMatchError $e) { echo $e->getMessage(), "\n"; }
