<?php
// Tricky precedence / associativity cases, checked against nikic/php-parser
// by tools/oracle (see crates/syntax/tests/README.md).

$r = $a . $b + $c;
$r = $a + $b . $c;
$r = $a << $b . $c;
$r = $a . $b << $c;
$r = -$a ** 2;
$r = $a ** -$b ** $c;
$r = !$a instanceof B;
$r = !$a == $b;
$r = $a ?? $b ?? $c;
$r = $a ?? $b ?: $c;
$r = $a ?: $b ?: $c;
$r = $a && $b || $c and $d or $e xor $f;
$r = $a = $b and $c;
$r = $x && $y = 5;
$r = !$a = foo();
$r = $a ? $b : ($c ? $d : $e);
$r = $a | $b ^ $c & $d;
$r = $a < $b == $c > $d;
$r = (int) $a + $b;
$r = @$a[1] . $b;
$r = $a++ + ++$b;
$r = clone $a->b;
$r = new Foo()->bar();
$r = print $a . $b;
$r = $a % $b * $c / $d;
$r = $a instanceof $b && $c;
$r = $a .= $b . $c;
$r = $a ??= $b ?? $c;
$r = fn($x) => $x + 1 . 'a';
$r = $a & $b == $c;
$r = $a - $b - $c + $d;
$r = $a / $b / $c;
$r = -$a * $b;
$r = ~$a + $b;
$r = $a <=> $b . $c;
$r = $a and print $b;
$r = $a?->b ?? $c;
$r = $a ? $b : $c ?? $d;
$r = isset($a) && !empty($b) || $c === null;
$r = $a !== $b ?: $c;
$r = (string) $a . $b;
$r = $a . -$b;
$r = $a * -$b ** 2;
$r = - -$a;
$r = !!$a;
$r = $a = $b = $c += 1;
[$a, [$b, $c]] = $d ?? [];
$r = match (true) { $a > 1, $b < 2 => $a + 1, default => $b ?: $c };
$r = $a->b()['c']::D + Foo::$e ** 2;
$r = "x{$a->b($c + 1)}y" . "$d[1] $e->f";
$r = throw $e ?? new E();
$r = include $a . '.php';
$r = $a ? $b = 1 : 2;
$r = -1 + -2.5 - +3;
$r = (object) ['a' => 1 + 2] ?: null;
$r = static fn(int ...$xs): int => array_sum($xs) * 2;
$r = strlen(...);
$r = $a->{$b . 'c'}($d) . $a::{$e}();

function gen()
{
    $r = yield $a + 1;
    $x = yield $k => $v . 'x';
    yield from $a ?? [];
}
