<?php
function fact($n) { return $n <= 1 ? "1" : bcmul_s($n, fact($n - 1)); }
function bcmul_s($a, $b) { return (string)($a * (int)$b); }
function walk($t, $d = 0) { $s = str_repeat(' ', $d) . $t['n'] . "\n"; foreach ($t['c'] ?? [] as $c) $s .= walk($c, $d + 1); return $s; }
class T { public $kids = []; function __construct(public $n) {} function add($k) { $this->kids[] = $k; return $this; }
  function depth() { $m = 0; foreach ($this->kids as $k) { $m = max($m, $k->depth()); } return $m + 1; }
  function names() { $o = [$this->n]; foreach ($this->kids as $k) { $o = array_merge($o, $k->names()); } return $o; } }
for ($i = 0; $i < 3; $i++) {
  echo fact(10), " ", walk(['n' => 'a', 'c' => [['n' => 'b', 'c' => [['n' => 'c']]], ['n' => 'd']]]);
  $t = (new T('r'))->add((new T('x'))->add(new T('y')))->add(new T('z'));
  echo $t->depth(), implode(',', $t->names()), "\n";
}
function thrower($n) { if ($n == 0) throw new Exception("deep"); return thrower($n - 1); }
try { thrower(50); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
echo thrower2(), "\n"; function thrower2() { try { return thrower(3); } catch (Exception $e) { return "caught"; } }
