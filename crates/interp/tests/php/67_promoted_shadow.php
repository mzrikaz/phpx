<?php
abstract class A { public function __construct(private string $output) {} public function a() { return $this->output; } }
class B extends A { private string $output; public function __construct(string $o) { parent::__construct($this->output = "B:$o"); } public function b() { return $this->output; } }
for ($i = 0; $i < 30; $i++) { $c = new B("x$i"); $r = $c->a() . '|' . $c->b(); } echo $r, "\n"; var_dump($c);
