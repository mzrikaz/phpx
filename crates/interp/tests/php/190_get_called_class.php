<?php
class A { static function who() { return get_called_class(); } function me() { return get_called_class(); } static function viaSelf() { return self::who(); } static function viaStatic() { return static::who(); } }
class B extends A {}
function f() { return get_called_class(); }
for ($i = 0; $i < 3; $i++) { var_dump(A::who(), B::who(), (new B)->me(), B::viaSelf(), B::viaStatic(), A::viaStatic()); try { f(); } catch (Error $e) { echo $e->getMessage(), "\n"; } $c = fn() => get_called_class(); try { var_dump($c()); } catch (Error $e) { echo $e->getMessage(), "\n"; } }
