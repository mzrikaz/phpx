<?php
interface Shape { public function area(): float; }
abstract class Base implements Shape { protected string $name; public int $hits = 0; public $plain;
    public function __construct(string $n) { $this->name = $n; $this->plain = $n; } public function name(): string { return $this->name; }
    public function touch(): int { $this->hits++; ++$this->hits; $this->hits--; return $this->hits; } }
final class Rect extends Base { public function __construct(private float $w, private float $h) { parent::__construct('rect'); } public function area(): float { return $this->w * $this->h; } }
final class Circle extends Base { public function __construct(private float $r) { parent::__construct('circle'); } public function area(): float { return 3.0 * $this->r * $this->r; } }
final class Tri extends Base { public function __construct(public readonly float $b, public readonly float $h) { parent::__construct('tri'); } public function area(): float { return $this->b * $this->h / 2; } }
final class Sq extends Base { public function __construct(private int|float $s) { parent::__construct('sq'); } public function area(): float { return $this->s * $this->s; } }
class Mag { private $d = []; function __get($k) { echo "get($k) "; return $this->d[$k] ?? null; } function __set($k, $v) { echo "set($k=" . json_encode($v) . ") "; $this->d[$k] = $v; } }
class Weird { public $n = null; public $s = 'a'; public $f = 1.5; public ?int $ni = null; public $big = PHP_INT_MAX; }
function total(array $shapes): array { $a = 0.0; $n = []; foreach ($shapes as $s) { $a += $s->area(); $n[] = $s->name() . $s->touch(); } return [$a, implode(',', $n)]; }
for ($i = 0; $i < 4; $i++) {
    $shapes = [new Rect(2, 3), new Circle(1), new Tri(4, 5), new Sq(3), new Rect(1.5, 2), new Sq(2.5)];
    echo json_encode(total($shapes)), "\n";
    $t = $shapes[2];
    try { $t->__construct(1, 1); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    $m = new Mag; $m->x++; $m->x++; echo "\n";
    $w = new Weird; $w->n++; $w->s++; $w->f++; $w->ni++; $w->big++; $w->n--; var_dump($w);
    $u = new Weird; unset($u->n); $u->n++; var_dump($u->n);
    $x = null; try { $x->p++; } catch (Error $e) { echo $e->getMessage(), "\n"; }
}
