<?php
class T { function r(?array $packages = null, bool $withCore = false) { var_dump($packages, $withCore); } }
class F { static function __callStatic($m, $a) { var_dump($a); return (new T)->$m(...$a); }
  function __call($m, $a) { var_dump($a); return (new T)->$m(...$a); } }
F::r(withCore: true);
(new F)->r(withCore: true);
F::r(['x'], withCore: true);
$a = ['withCore' => true]; (new T)->r(...$a);
call_user_func_array([new T, 'r'], ['withCore' => true]);
