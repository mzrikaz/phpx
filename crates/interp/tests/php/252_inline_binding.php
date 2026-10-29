<?php
// Inline parameter binding (class types, literal defaults), promoted
// constructor parameters and typed property writes in compiled code.
interface Shape {}
final class Sq implements Shape { public function __construct(public float $side = 1) {} }
final class Ci implements Shape { public function __construct(public readonly float $r) {} }
class Other {}
final class Node {
    public function __construct(public ?Node $l = null, public ?Node $r = null) {}
}
class Base { public function __construct(private int $id, protected string $tag = 'b') {} public function id() { return $this->id; } }
class Kid extends Base { private $id = 'kid'; public function __construct(int $id) { parent::__construct($id, 'k'); } public function both() { return [$this->id, $this->id(), $this->tag]; } }
class Holder { public ?Shape $s = null; public Node $n; }

function kind(Shape $s): string { return get_class($s); }
function opt(?Shape $s = null, int $n = 3, float $x = 2, bool $b = true, $u = -1, string $t = 'q') { return [$s === null ? null : get_class($s), $n, $x, $b, $u, $t]; }
function count_nodes(Node $n): int { return $n->l === null ? 1 : 1 + count_nodes($n->l) + count_nodes($n->r); }
function make(int $d): Node { return $d === 0 ? new Node() : new Node(make($d - 1), make($d - 1)); }
function isnull($v) { return [$v === null, $v !== null, null === $v, null !== $v]; }

$out = [];
for ($round = 0; $round < 1500; $round++) {
    $s = $round % 2 ? new Sq(2) : new Ci(3);
    $k = kind($s);
    $o = opt($round % 3 ? $s : null, $round % 5);
    $c = count_nodes(make($round % 4));
    if ($round % 500 === 7) {
        $out[] = [$k, $o, $c, opt(), opt(new Sq, 1, 2.5, false, 'u', 'z')];
    }
}
echo json_encode($out), "\n";

// Values of the wrong type still go through PHP's coercion and errors.
echo json_encode(opt(null, "7", 1)), "\n";
foreach ([fn() => kind(new Other), fn() => opt(new Other), fn() => count_nodes(null), fn() => opt(null, 'x')] as $f) {
    try { $f(); } catch (TypeError $e) { echo preg_replace('/, called in.*/', '', $e->getMessage()), "\n"; }
}

// Promotion: coercion, readonly, private shadowing, interpreted callers.
$sq = new Sq(4); var_dump($sq->side);
$ci = new Ci(5); var_dump($ci->r);
try { $ci->__construct(6); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($ci->r);
echo json_encode((new Kid(9))->both()), "\n";
$cls = 'Node'; $n = new $cls(new Node, null); var_dump($n->l instanceof Node, $n->r);
$n2 = (new ReflectionClass('Node'))->newInstance(null, new Node); var_dump($n2->l, $n2->r instanceof Node);

// Typed property writes: null, accepted class, another class, wrong type.
$h = new Holder;
for ($i = 0; $i < 3000; $i++) { $h->s = $i % 3 === 0 ? null : ($i % 3 === 1 ? new Sq : new Ci(1.5)); $h->n = new Node; }
var_dump(get_class($h->s));
try { $h->s = new Other; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $h->n = null; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$r = null; $ref = &$r;
echo json_encode([isnull(null), isnull(0), isnull(''), isnull(false), isnull([]), isnull($ref)]), "\n";
