<?php
class C implements ArrayAccess { private $d = ['view' => ['paths' => ['x']]];
  function offsetExists($k): bool { echo "exists $k\n"; return isset($this->d[$k]); }
  function offsetGet($k): mixed { echo "get $k\n"; return $this->d[$k]; }
  function offsetSet($k, $v): void {} function offsetUnset($k): void {} }
class App { function __get($k) { echo "__get $k\n"; return $k === 'config' ? new C : null; } }
$a = new App;
var_dump(isset($a->config['view']['paths']));
var_dump(isset($a->config));
var_dump($a->config ?? 'dflt');
var_dump($a->missing ?? 'dflt');
var_dump(isset($a->config->x));
var_dump(empty($a->config));
