<?php
$config = ['a' => 1];
$count = 0;
function bump() { global $count, $config; $count++; $config['b'] = $count; return $count; }
function peek() { return $GLOBALS['count'] . ':' . count($GLOBALS['config']); }
function setg() { $GLOBALS['made'] = 'yes'; $GLOBALS['config']['c'] = 3; }
for ($i = 0; $i < 3; $i++) { bump(); echo peek(), "\n"; $count += 10; }
setg(); echo $made, " ", json_encode($config), "\n";
$name = 'dyn'; $$name = 'vv'; echo $dyn, " ", ${'dy' . 'n'}, " ", "{$name} {$$name}\n";
$arr = ['k' => 'v']; $vn = 'arr'; echo $$vn['k'], "\n";
function ex() { extract(['p' => 1, 'q' => 2]); extract(['p' => 9], EXTR_SKIP); echo $p + $q, "\n"; $n = extract(['x' => 1], EXTR_PREFIX_ALL, 'pre'); echo $pre_x, $n, "\n"; }
ex();
echo eval('return 6*7;'), "\n";
$z = 5; eval('$z = $z * 2; function fromeval() { return "fe"; }'); echo $z, fromeval(), "\n";
function inner() { $loc = 1; eval('$loc += 41;'); return $loc; } echo inner(), "\n";
$r = @$undefined_thing; var_dump($r);
$r = @file_get_contents('/nonexistent/x'); var_dump($r);
function gu() { unset($GLOBALS['made']); } gu(); var_dump(isset($made));
function gref() { global $refd; $refd = [1]; $refd[] = 2; } gref(); echo count($refd), "\n";
echo isset($GLOBALS['count']) ? "set\n" : "unset\n";
try { eval('this is bad'); } catch (ParseError $e) { echo "parse error caught\n"; }
$total = 0; $log = [];
function add($n) { global $total, $log; $total += $n; $log[] = $n; }
function show() { return $GLOBALS['total']; }
for ($i = 1; $i <= 5; $i++) { add($i); $total *= 2; echo show(), " "; }
echo "\n", $total, " ", count($log), "\n";
