<?php
$m = new WeakMap; $a = new stdClass; $b = new stdClass;
$m[$a] = 1; $m[$b] = [2]; var_dump(count($m), $m[$a], isset($m[$b]), isset($m[new stdClass]));
foreach ($m as $k => $v) { var_dump($k === $a || $k === $b, $v); }
unset($a); var_dump(count($m));
unset($m[$b]); var_dump(count($m));
try { $m[$b]; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $m['x'] = 1; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$o = new stdClass; $r = WeakReference::create($o); var_dump($r->get() === $o, $r === WeakReference::create($o)); unset($o); var_dump($r->get());
