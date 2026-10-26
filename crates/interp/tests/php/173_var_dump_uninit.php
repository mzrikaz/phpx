<?php
class T { public int $a; public ?string $b; protected array $c; private int|string $d; public $e; public static int $s = 1;
  function __construct() { unset($this->e); } }
$t = new T; var_dump($t); $t->a = 5; unset($t->b); var_dump($t);
class U extends T { public float $f; } var_dump(new U);
