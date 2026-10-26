<?php
// array_merge shapes: lists, string-keyed first array, mixed keys, refs.
var_dump(array_merge());
var_dump(array_merge([1, 2], [3], []));
var_dump(array_merge(['a' => 1, 'b' => 2], ['b' => 3, 5 => 'x', 'c' => 4], [7 => 'y']));
var_dump(array_merge([3 => 'a', 'k' => 'b'], ['k' => 'c', 'd']));
$v = 1; $r = ['x' => &$v, 'y' => 2]; $m = array_merge($r, ['z' => 3]); $v = 9; var_dump($m);
$h = ['a' => 1]; unset($h['a']); $h['b'] = 2; $m = array_merge($h, [10 => 'n']); $m[] = 'next'; var_dump($m);
// Copy-on-write of hash arrays, with and without shared references.
$a = ['a' => 1, 'b' => 2, 'c' => 3];
$b = $a; $b['d'] = 4; $b['a'] = 0; var_dump($a, $b);
$x = 5; $c = ['p' => &$x, 'q' => 1]; $d = $c; $d['p'] = 6; var_dump($x, $c['p']);
$e = ['p' => 1]; $ref = &$e['p']; $f = $e; unset($ref); $f['p'] = 2; var_dump($e['p'], $f['p']);
// Lookups: small and large hash arrays, numeric-string keys.
$big = []; for ($i = 0; $i < 20; $i++) { $big["k$i"] = $i; } $big['7'] = 'seven';
foreach (['k3', 'k19', 'k20', '7', 7, '07'] as $k) { var_dump(isset($big[$k]), array_key_exists($k, $big), $big[$k] ?? null); }
$small = ['1' => 'one', 'x' => null, -2 => 'neg'];
foreach ([1, '1', 'x', '-2', -2, 'y'] as $k) { var_dump(isset($small[$k]), array_key_exists($k, $small), $small[$k] ?? 'none'); }
// instanceof through self/static/parent from different classes at one site.
interface I {} class A implements I { function t($o) { return [$o instanceof self, $o instanceof static, $o instanceof I]; } }
class B extends A { function u($o) { return $o instanceof parent; } } class C {}
foreach ([new A, new B, new C] as $o) { var_dump((new A)->t($o), (new B)->t($o), (new B)->u($o)); }
function isA($o) { return $o instanceof A; }
for ($i = 0; $i < 3; $i++) { foreach ([new A, new B, new C, 'A', null] as $o) { echo isA($o) ? 'y' : 'n'; } } echo "\n";
