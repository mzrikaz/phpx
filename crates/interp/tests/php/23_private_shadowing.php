<?php
class A { private $q = 'A'; protected $p = 'pA'; public $pub = 1;
  function getA() { return $this->q; } function setA($v) { $this->q = $v; } function issetA() { return isset($this->q); }
  function dumpA() { return get_object_vars($this); } }
class B extends A { private $q = 'B'; public $p = 'pB';
  function getB() { return $this->q; } function setB($v) { $this->q = $v; } function dumpB() { return get_object_vars($this); } }
$b = new B;
for ($i = 0; $i < 3; $i++) { echo $b->getA(), $b->getB(), " "; $b->setA("a$i"); $b->setB("b$i"); }
echo "\n"; var_dump($b); print_r($b); echo "\n";
echo json_encode($b->dumpA()), json_encode($b->dumpB()), json_encode(get_object_vars($b)), "\n";
echo str_replace("\0", '\0', serialize($b)), "\n";
$u = unserialize(serialize($b)); echo $u->getA(), $u->getB(), "\n";
echo str_replace("\0", '\0', json_encode(array_keys((array)$b))), "\n";
$c = clone $b; $c->setA('cA'); echo $b->getA(), $c->getA(), $c->getB(), "\n";
foreach ($b as $k => $v) echo "$k=$v ";
echo "\n"; var_dump($b->issetA());
