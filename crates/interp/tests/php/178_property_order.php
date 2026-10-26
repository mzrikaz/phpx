<?php
trait T1 { public $t1a; protected $t1b; }
trait T2 { public $t2a; }
class A { use T2; public $a1; private $apriv; protected static $as; public $a2; }
class B extends A { use T1; public $b1; public static $bs; public $a1; }
class C extends B { public $c1; }
foreach ([A::class, B::class, C::class] as $c) { echo $c, ": ", implode(",", array_map(fn($p) => $p->getName(), (new ReflectionClass($c))->getProperties())), "\n"; }
echo implode(",", array_map(fn($p) => $p->getName(), (new ReflectionClass('C'))->getProperties(ReflectionProperty::IS_PUBLIC))), "\n";
echo implode(",", array_keys(get_class_vars('C'))), "\n";
echo implode(",", array_keys(get_object_vars(new C))), "\n";
var_dump(new C);
foreach ((new ReflectionClass('C'))->getMethods() as $m) {}
