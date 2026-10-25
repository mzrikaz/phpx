<?php
class App { public $sp = ['a' => 1, 'b' => 2]; public $n = 0;
  function boot() { array_walk($this->sp, function ($v, $k) { $this->n++; if ($k === 'a') $this->sp['c'] = 3; }); return [$this->sp, $this->n]; }
  function mod() { array_walk($this->sp, function (&$v) { $v *= 10; }); return $this->sp; } }
$a = new App; var_dump($a->boot(), $a->mod());
$x = [3, 1, 2]; $o = new stdClass; $o->arr = [3, 1, 2]; sort($o->arr); var_dump($o->arr);
