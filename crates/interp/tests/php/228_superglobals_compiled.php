<?php
// Superglobals and $GLOBALS['name'] in compiled functions: reads, quiet
// reads, element writes, compound updates, unset, by-reference args.
function env($k) { return $_ENV[$k] ?? $_SERVER[$k] ?? 'none'; }
function setsv($k, $v) { $_SERVER[$k] = $v; $_SERVER['nested'][$k] = $v; $GLOBALS['gv'][$k] = $v; }
function uns($k) { unset($_SERVER[$k], $_SERVER['nested'][$k]); }
function has($k) { return [isset($_SERVER[$k]), empty($_SERVER[$k]), isset($_SERVER['nested'][$k]), array_key_exists($k, $_SERVER)]; }
function count_g() { return count($GLOBALS['gv'] ?? []); }
function whole() { $_ENV = ['A' => 1]; return $_ENV; }
function byref(&$a) { $a['x'] = 1; }
function refuse() { byref($_SERVER['refd']); return $_SERVER['refd']; }
function undef() { return $GLOBALS['nope']; }
function uninc() { $_SERVER['cnt'] ??= 0; $_SERVER['cnt']++; $_SERVER['s'] = ($_SERVER['s'] ?? '') . 'a'; $_SERVER['s'] .= 'b'; return [$_SERVER['cnt'], $_SERVER['s']]; }
function closure_ref() { $arr = ['a' => ['b' => 1, 'c' => 2], 'd' => 3]; $f = function () use (&$arr) { unset($arr['d']); unset($arr['a']['b']); }; for ($i = 0; $i < 3; $i++) $f(); return $arr; }
$gv = [];
for ($i = 0; $i < 1500; $i++) {
    $k = 'K' . ($i % 3);
    setsv($k, $i);
    $h = has($k);
    if ($i % 2) uns($k);
    $e = env('PATH_NOPE');
}
var_dump(env('K1') , has('K0'), has('K1'), count_g(), count($gv), $_SERVER['nested'], whole(), $_ENV);
for ($i = 0; $i < 1500; $i++) { $r = refuse(); $u = uninc(); $c = closure_ref(); }
var_dump($r, $u[0], strlen($u[1]), $c);
set_error_handler(function ($n, $m) { static $c = 0; if ($c++ < 2) echo "handler: $m\n"; return true; });
for ($i = 0; $i < 1500; $i++) $x = undef();
var_dump($x);
unset($_SERVER['refd']); var_dump(isset($_SERVER['refd']));
// unset() through an element that is a reference (compiled).
function midref() { $inner = ['b' => 1, 'c' => 2, 'a' => ['b' => 9]]; $x = ['a' => &$inner]; unset($x['a']['b']); return $inner; }
for ($i = 0; $i < 1500; $i++) $m = midref();
var_dump($m);
