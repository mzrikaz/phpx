<?php
namespace Foo { function count($x) { return 0; } }
namespace {
function id($x) { return $x; }
class K { private $k = null;
  function keys() { return $this->k ??= [1, 2]; }
  function plain() { return [1, 2, 3]; }
  function a() { return count($this->plain()); }
  function b() { return count($this->keys()); }
  function c() { $x = $this->keys(); return count($x); }
  function d() { return id($this->keys()); } }
$o = new K; for ($i = 0; $i < 3; $i++) { echo $o->a(), " "; echo json_encode($o->d()), " "; echo $o->c(), " "; echo $o->b(), "\n"; }
}
