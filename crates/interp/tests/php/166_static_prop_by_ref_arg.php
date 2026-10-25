<?php
class E { protected static $c = [];
  function start($x) { static::$c[] = $x; }
  function stop() { array_pop(static::$c); }
  static function cur() { return end(static::$c); }
  static function is() { return ! empty(static::$c); } }
$e = new E; $e->start('a'); var_dump(E::is(), E::cur()); $e->start('b'); var_dump(E::cur()); $e->stop(); var_dump(E::cur()); $e->stop(); var_dump(E::is());
class F { static $a = [3,1,2]; static $m; static function go() { sort(static::$a); preg_match('/(b)/', 'abc', self::$m); return [self::$a, self::$m]; } }
var_dump(json_encode(F::go()));
function by(&$x) { $x[] = 9; } class G { static $l = []; static function go() { by(self::$l); return self::$l; } } var_dump(G::go());
