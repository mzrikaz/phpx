<?php
// binary-trees: allocation and recursion.
final class Node { public function __construct(public ?Node $l = null, public ?Node $r = null) {} }
function make(int $d): Node { return $d === 0 ? new Node() : new Node(make($d - 1), make($d - 1)); }
function check(Node $n): int { return $n->l === null ? 1 : 1 + check($n->l) + check($n->r); }
$max = 16; $t = hrtime(true); $out = [];
$out[] = check(make($max + 1));
$long = make($max);
for ($d = 4; $d <= $max; $d += 2) { $it = 1 << ($max - $d + 4); $c = 0; for ($i = 0; $i < $it; $i++) $c += check(make($d)); $out[] = $c; }
$out[] = check($long);
printf("bintrees %.0f ms %s\n", (hrtime(true) - $t) / 1e6, implode(',', $out));
