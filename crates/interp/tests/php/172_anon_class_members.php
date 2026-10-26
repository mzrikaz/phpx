<?php
abstract class Comp { public $base = 1; }
$o = new class extends Comp { public $forMount; private $secret = 2; protected int $t; function s() { return $this->secret; } };
foreach ((new ReflectionObject($o))->getProperties() as $p) { echo $p->getName(), ' ', strstr($p->getDeclaringClass()->getName(), "\0", true) ?: $p->getDeclaringClass()->getName(), ' ', $p->getDeclaringClass()->getName() === get_class($o) ? 'self' : 'other', "\n"; }
var_dump($o, $o->s());
print_r($o); echo "\n";
var_dump(array_map(fn($k) => str_replace("\0", '\0', strstr($k, "\0", false) ? preg_replace('/\/.*\$\w+/', 'X', $k) : $k), array_keys((array)$o)));
try { $o->t; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $o->secret; } catch (Error $e) { echo $e->getMessage(), "\n"; }
