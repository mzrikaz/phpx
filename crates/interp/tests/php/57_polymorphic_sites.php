<?php
// One call site, a recursive call through it with another class, then a
// private property read in the second class's method (compiled code).
abstract class Node { abstract public function walk(int $depth): string; }
function visit(Node $n, int $depth): string { return $n->walk($depth); }
class Outer extends Node {
    private string $tag = 'outer';
    public function walk(int $depth): string { return $this->tag . ($depth > 0 ? '>' . visit(new Inner, $depth - 1) : ''); }
}
class Inner extends Node {
    private string $secret = 'inner';
    public function walk(int $depth): string { return $this->secret . ($depth > 0 ? '>' . visit(new Outer, $depth - 1) : ''); }
}
$out = '';
for ($i = 0; $i < 300; $i++) {
    $out = visit($i % 2 ? new Inner : new Outer, 3);
}
echo $out, "\n";
