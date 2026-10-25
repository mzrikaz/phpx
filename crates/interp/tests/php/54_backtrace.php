<?php
namespace App;
class Model {
    public function belongsTo($related) { return $this->guess(); }
    protected function guess() { [$one, $two, $caller] = debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS, 3); return $caller['function']; }
    public function user() { return $this->belongsTo('User'); }
    public static function make(int $a, string $s) { return debug_backtrace(); }
}
function helper($x) { return (new Model)->user(); }
echo helper(5), "\n";
$t = Model::make(1, 'abc');
echo count($t), ' ', $t[0]['function'], ' ', $t[0]['class'], ' ', $t[0]['type'], ' ', $t[0]['line'], ' ', json_encode($t[0]['args']), "\n";
function thrower($a) { throw new \RuntimeException("boom"); }
function outer() { thrower([1, 2]); }
try { outer(); } catch (\Exception $e) {
    echo preg_replace('~/\S*?\.php~', 'F', $e->getTraceAsString()), "\n";
    foreach ($e->getTrace() as $f) echo $f['function'], '@', $f['line'], ' ';
    echo "\n";
}
$c = function ($v) { return debug_backtrace(); };
$r = [$c(1)];
echo count($r[0]), "\n";
function lvl() { ob_start(); debug_print_backtrace(); echo preg_replace('~/\S*?\.php~', 'F', ob_get_clean()); }
function lvl2() { lvl(); }
lvl2();
$e2 = new \LogicException('outer', 3, new \InvalidArgumentException('inner'));
echo preg_replace('~/\S*?\.php~', 'F', (string) $e2), "\n";
$ee = new \ErrorException('warn', 0, E_WARNING, 'x.php', 7); echo $ee->getSeverity(), ' ', $ee->getFile(), ' ', $ee->getLine(), "\n";
$v = new \Exception('v'); var_dump($v->getMessage(), $v->getTrace());
