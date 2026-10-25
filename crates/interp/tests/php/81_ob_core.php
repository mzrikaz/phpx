<?php
ob_start();
echo "hello";
var_dump(ob_get_length());
function wrap($b, $phase) { return "[" . strtoupper($b) . ":$phase]"; }
ob_start('wrap');
echo "abc";
ob_flush();
echo "def";
ob_clean();
echo "ghi";
$h = ob_list_handlers();
$st = ob_get_status(true);
ob_end_flush();
$all = ob_get_clean();
var_dump($all, $h);
foreach ($st as $s) { echo $s['name'], ' ', $s['type'], ' ', $s['level'], ' ', $s['chunk_size'], ' ', $s['buffer_used'], "\n"; }
ob_start(function ($b) { return false; });
echo "passthrough\n";
$x = ob_get_flush();
var_dump($x);
var_dump(ob_get_level(), ob_get_status(), ob_get_length(), @ob_end_clean());
function upper($s) { return strtoupper($s); }
ob_start('upper');
echo "via named function\n";
ob_end_flush();
ob_start();
echo "unflushed at exit through handler\n";

class P { private $a = 1; protected $b = 2; public $c = 3; }
$m = get_mangled_object_vars(new P);
foreach ($m as $k => $v) echo json_encode($k), "=$v\n";
interface MyI {}
trait MyT {}
class MyC implements MyI { use MyT; }
enum MyE {}
$c = get_declared_classes(); $i = get_declared_interfaces(); $t = get_declared_traits();
var_dump(array_slice($c, -2), end($i), end($t), in_array('stdClass', $c), in_array('ArrayAccess', $i));
var_dump(count(get_included_files()), zend_version());
class A { public static function who() { return static::class; } public static function test() { return forward_static_call([A::class, 'who']); } }
class B extends A {}
var_dump(forward_static_call_array('str_repeat', ['ab', 2]));
