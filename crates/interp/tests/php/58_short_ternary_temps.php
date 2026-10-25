<?php
class M { public $global = []; public $t = false;
  function a() { return array_filter(['a', $this->t ? 'T' : null]); }
  function b() { return $this->global ?: ['a']; }
  function c() { return $this->global ?: array_filter(['a', null]); }
  function d() { return $this->global ?: array_filter(['a', $this->t ? 'T' : null]); }
  function e() { $x = $this->global; return $x ?: array_filter(['a', $this->t ? 'T' : null]); }
  function f() { return $this->global ?: count(['a', $this->t ? 'T' : null]); }
}
$o = new M; foreach (['a','b','c','d','e','f'] as $m) { echo $m, ':'; try { echo json_encode($o->$m()); } catch (Throwable $e) { echo get_class($e); } echo ' '; } echo "\n";
