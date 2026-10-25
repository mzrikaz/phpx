<?php
class S { public $a = [1, 2, 3]; public $b = ['x' => [3, 1, 2]];
  function pop() { return array_pop($this->a); }
  function sortit() { sort($this->b['x']); return $this->b['x']; }
  function push() { array_push($this->a, 9); $this->a[] = 10; return count($this->a); } }
$s = new S;
for ($i = 0; $i < 3; $i++) { echo $s->pop(), " ", count($s->a), " ", implode(",", $s->sortit()), " ", $s->push(), "\n"; }
$arr = ['k' => [5, 4]];
for ($i = 0; $i < 2; $i++) { sort($arr['k']); echo implode(",", $arr['k']), " "; array_shift($arr['k']); echo count($arr['k']), "\n"; }
function f() { static $st = [3,2,1]; sort($st); return implode(",", $st); }
echo f(), f(), "\n";
function addOne(&$x) { $x[] = count($x); }
function inc(&$n) { $n++; }
function plain($x) { $x[] = 99; return count($x); }
class H { public $list = []; public $n = 0; public static $s = [1];
  function run() { addOne($this->list); inc($this->n); $this->bump($this->list); return count($this->list) . "/" . $this->n . "/" . plain($this->list); }
  function bump(array &$a) { $a[] = 'b'; }
  static function sinc(&$v) { $v++; } }
$h = new H;
for ($i = 0; $i < 3; $i++) echo $h->run(), " ";
echo "\n";
$data = ['a' => ['n' => 1]];
for ($i = 0; $i < 3; $i++) { inc($data['a']['n']); H::sinc($data['a']['n']); }
echo $data['a']['n'], "\n";
$o = new stdClass; $o->v = 5; inc($o->v); echo $o->v, "\n";
$m = 'inc'; $q = ['z' => 1]; $m($q['z']); echo $q['z'], "\n";
preg_match('/(\d+)/', 'abc 42', $h->list); echo implode("|", $h->list), "\n";
