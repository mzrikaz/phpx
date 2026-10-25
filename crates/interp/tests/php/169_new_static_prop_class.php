<?php
class B { function __construct(public $q = null) {} }
class C extends B {}
class M { protected static $builder = B::class; public static $arr = ['k' => C::class];
  function make($q) { return new static::$builder($q); }
  function make2() { return new self::$arr['k']; }
  static function make3() { return new M::$builder; } }
class N extends M { protected static $builder = C::class; }
var_dump((new M)->make(1), (new N)->make(2), (new M)->make2(), M::make3());
