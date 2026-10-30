<?php
// One typed-property write site serving two (or more) classes.
abstract class Base { protected string $name; protected ?int $n = null; protected float $f = 0.0;
  function __construct(string $n, $k) { $this->name = $n; $this->n = $k; $this->f = $k * 1.5; } function get() { return [$this->name, $this->n, $this->f]; } }
class A extends Base { public $a = 1; function __construct($k) { parent::__construct('a', $k); } }
class B extends Base { public $b1 = 1; public $b2 = 2; function __construct($k) { parent::__construct('b', $k); } }
class C extends Base { function __construct($k) { parent::__construct('c', $k); } }
class D extends Base { private string $name2 = 'x'; }
for ($r = 0; $r < 3; $r++) {
    $out = [];
    for ($i = 0; $i < 6; $i++) {
        $o = match ($i % 4) { 0 => new A($i), 1 => new B($i), 2 => new C($i), 3 => new D("d$i", $i % 2 ? null : $i) };
        $out[] = $o->get();
    }
    try { new D(5, 1); } catch (TypeError $e) { $out[] = $e->getMessage(); }
    try { new D("s", "x"); } catch (TypeError $e) { $out[] = $e->getMessage(); }
    $out[] = (new D("s", "7"))->get();
    echo json_encode($out), "\n";
}
