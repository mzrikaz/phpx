<?php
class Base {}
$o = new class extends Base {};
$n = get_class($o);
var_dump(strlen($n) > 20, str_contains($n, "\0"), class_exists($n), is_subclass_of($n, 'Base'), $o::class === $n, $n === (new ReflectionClass($o))->getName());
var_dump(strstr($n, "\0", true));
echo json_encode(explode("\0", $n)[0]), "\n";
$x = new $n; var_dump($x instanceof Base, get_class($x) === $n);
var_dump($o);
print_r($o); echo "\n";
echo (new ReflectionClass($o))->getShortName() === $n ? "same" : "diff", "\n";
var_dump((new ReflectionClass($o))->isAnonymous());
$f = function() { return static::class; };
