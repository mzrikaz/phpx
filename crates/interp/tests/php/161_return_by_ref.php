<?php
function &g() { static $v = 1; return $v; }
function n() { return 5; }
class C { public $a = [1]; public static $s = 0;
  function &get() { return $this->a; }
  static function &st() { return self::$s; }
  function &el($k) { return $this->a[$k]; } }
$x = &g(); $x++; var_dump(g());
$y = &n(); $y++; var_dump($y);
$c = new C; $r = &$c->get(); $r[] = 2; var_dump($c->a);
$s = &C::st(); $s = 9; var_dump(C::$s);
$e = &$c->el(5); $e = 'new'; var_dump($c->a);
$z = g(); $z++; var_dump(g());
function &arr() { static $a = []; return $a; }
$q = &arr(); $q['k'] = 1; var_dump(arr());
foreach ([1,2] as $i) { $w = &g(); $w += 10; } var_dump(g());
$m = new WeakMap; $o = new stdClass; $p = new stdClass;
$m[$o] = [];
$m[$o][] = 1;
$m[$o]['k'] = 2;
$m[$p] = [3];
$m[$o]['k'] .= 'x';
$m[$o]['n']['deep'] = 4;
$r = &$m[$o];
$r[] = 5;
var_dump($m[$o], $m[$p], count($m));
$s = new SplObjectStorage; $s[$o] = [1]; $s[$o][] = 2; var_dump($s[$o]);
$a = new ArrayObject(['x' => []]); $a['x'][] = 1; var_dump($a['x']);
class U2 implements ArrayAccess { public $d = ['a' => []];
 function offsetExists($k): bool { return isset($this->d[$k]); }
 function offsetGet($k): mixed { echo "get $k\n"; return $this->d[$k] ?? null; }
 function offsetSet($k, $v): void { echo "set $k\n"; $this->d[$k] = $v; }
 function offsetUnset($k): void {} }
class R2 extends U2 { function &offsetGet($k): mixed { echo "rget $k\n"; return $this->d[$k]; } }
$u = new U2; $u['a'][] = 1; var_dump($u->d);
$r = new R2; $r['a'][] = 1; $r['b']['x'] = 2; var_dump($r->d);
$a = new ArrayObject([]); $a['new'][] = 1; $a['n2']['k'] = 2; var_dump($a->getArrayCopy());
class AO extends ArrayObject { function offsetGet($k): mixed { echo "aoget\n"; return parent::offsetGet($k); } }
$b = new AO(['x'=>[]]); $b['x'][] = 1; var_dump($b->getArrayCopy());
$it = new ArrayIterator(['x'=>[]]); $it['x'][] = 9; var_dump($it['x']);
function run() {
  $x = &g(); $x++;
  $c = new C; $r = &$c->get(); $r[] = 2;
  $m = new WeakMap; $o = new stdClass; $m[$o] = []; $m[$o][] = 1; $w = &$m[$o]; $w[] = 2;
  $ao = new ArrayObject([]); $ao['k'][] = 1;
  return [g(), $c->a, $m[$o], $ao['k']];
}
for ($i = 0; $i < 3; $i++) $res = run();
echo json_encode($res), "\n";
