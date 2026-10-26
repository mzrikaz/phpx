<?php
// First writes to never-initialized typed properties (constructors), through the write cache.
class P {
    private array $a; protected ?P $next; public int $n; public string|int $u;
    function __construct(array $a, ?P $next, $n) { $this->a = $a; $this->next = $next; $this->n = $n; $this->u = $n; }
    function get() { return [$this->a, $this->next?->n, $this->n, $this->u]; }
}
class M { public int $x; public function __set($k, $v) { echo "__set $k\n"; } public function __get($k) { return "__get $k"; } }
$last = null;
for ($i = 0; $i < 3000; $i++) { $last = new P([$i], $last instanceof P && $i % 2 ? $last : null, $i); }
var_dump($last->get());
try { new P([1], null, 'x'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$p = new P([1], null, '7'); var_dump($p->get()[2]);   // coerced
for ($i = 0; $i < 2000; $i++) { $m = new M; $m->x = $i; }
var_dump($m->x);
$m = new M; unset($m->x); $m->x = 5;   // after unset(): magic applies
try { var_dump($m->x); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
class N { public int $x = 1; public ?string $s = null; public function __get($k) { return $k === 'x' ? '42' : 7; } }
$n = new N; unset($n->x, $n->s); var_dump($n->x, $n->s);
$m2 = new M; var_dump(isset($m2->x)); try { echo $m2->x; } catch (Error $e) { echo $e->getMessage(), "\n"; }
