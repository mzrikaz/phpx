<?php
// A method call site serves two classes directly (the second entry);
// a third class, other calling scopes and recursion through either entry
// still behave as in PHP.
interface Node { public function sum(int $depth): int; }
final class Leaf implements Node { public function __construct(private int $v) {} public function sum(int $depth): int { return $this->v * $depth; } }
final class Pair implements Node {
    public function __construct(private Node $a, private Node $b) {}
    public function sum(int $depth): int { return $this->a->sum($depth + 1) + $this->b->sum($depth + 1); }
}
final class Neg implements Node { public function __construct(private Node $n) {} public function sum(int $depth): int { return -$this->n->sum($depth); } }
class P { private function secret() { return 'P'; } public function call(P $o) { return $o->secret(); } }
class Q extends P { private function secret() { return 'Q'; } public function call(P $o) { return parent::call($o) . $o->who(); } public function who() { return 'q'; } }
class R extends P { public function who() { return 'r'; } }

function build(int $d, int $i): Node {
    if ($d === 0) return new Leaf($i);
    $p = new Pair(build($d - 1, $i), build($d - 1, $i + 1));
    return $d % 3 === 0 ? new Neg($p) : $p;
}
$t = 0;
for ($i = 0; $i < 300; $i++) { $t += build($i % 6, $i)->sum(1); }
echo $t, "\n";
$shapes = [new Leaf(1), new Pair(new Leaf(2), new Leaf(3)), new Neg(new Leaf(4))];
$t = 0;
for ($i = 0; $i < 3000; $i++) { $t += $shapes[$i % 2]->sum(2) + $shapes[$i % 3]->sum(1); }
echo $t, "\n";
$out = [];
$objs = [new P, new Q, new R];
for ($i = 0; $i < 1500; $i++) {
    $a = $objs[$i % 3]; $b = $objs[($i + 1) % 3];
    try { $r = $a->call($b); } catch (Error $e) { $r = $e->getMessage(); }
    if ($i < 6) $out[] = $r;
}
echo json_encode($out), "\n";
