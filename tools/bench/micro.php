<?php
function b_empty($N) { for ($i = 0; $i < $N; $i++) { } }
function b_assign($N) { $x = 0; for ($i = 0; $i < $N; $i++) { $x = $x + 1; } return $x; }
function b_push($N) { $a = []; for ($i = 0; $i < $N; $i++) { $a[] = $i; } return $a; }
function b_iter($a) { $s = 0; foreach ($a as $v) { $s += $v; } return $s; }
function b_builtin($N) { $y = 0; for ($i = 0; $i < $N; $i++) { $y = abs($i); } return $y; }
function id($v) { return $v; }
function b_ucall($N) { $y = 0; for ($i = 0; $i < $N; $i++) { $y = id("s"); } return $y; }
function b_concat($N) { $s = ''; for ($i = 0; $i < $N; $i++) { $s .= 'x'; } return strlen($s); }
function b_index($a, $N) { $y = 0; for ($i = 0; $i < $N; $i++) { $y = $a[$i]; } return $y; }
class P { public $p = 1; public function get() { return $this->p; } }
function b_prop($o, $N) { $y = 0; for ($i = 0; $i < $N; $i++) { $y = $o->p; } return $y; }
function b_method($o, $N) { $y = 0; for ($i = 0; $i < $N; $i++) { $y = $o->get(); } return $y; }
function b_str($N) { $s = ''; for ($i = 0; $i < $N; $i++) { $s = "item $i"; } return $s; }
$N = 1000000;
$r = [];
$t = microtime(true); b_empty($N); $r['empty'] = microtime(true) - $t;
$t = microtime(true); b_assign($N); $r['assign'] = microtime(true) - $t;
$t = microtime(true); $a = b_push($N); $r['push'] = microtime(true) - $t;
$t = microtime(true); b_iter($a); $r['iter'] = microtime(true) - $t;
$t = microtime(true); b_builtin($N); $r['builtin'] = microtime(true) - $t;
$t = microtime(true); b_ucall($N); $r['ucall'] = microtime(true) - $t;
$t = microtime(true); b_concat($N); $r['concat'] = microtime(true) - $t;
$t = microtime(true); b_index($a, $N); $r['index'] = microtime(true) - $t;
$o = new P;
$t = microtime(true); b_prop($o, $N); $r['prop'] = microtime(true) - $t;
$t = microtime(true); b_method($o, $N); $r['method'] = microtime(true) - $t;
$t = microtime(true); b_str($N); $r['interp'] = microtime(true) - $t;
foreach ($r as $k => $v) printf("%-8s %7.1f ns\n", $k, $v * 1e9 / $N);
