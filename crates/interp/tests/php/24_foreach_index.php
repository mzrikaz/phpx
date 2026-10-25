<?php
function f1($a) { $o = []; foreach ($a as $k => $v) { $o[] = "$k:$v"; if ($k == 1) { $a[] = 'added'; $a[0] = 'changed'; } } return implode(',', $o) . ' / ' . implode(',', $a); }
function f2() { $x = 1; $a = [1, &$x, 3]; $o = []; foreach ($a as $v) { $o[] = $v; $x = 9; } return implode(',', $o); }
function f3($a) { $s = 0; foreach ($a as $v) { if ($v > 3) break; $s += $v; } foreach ($a as $v) { if ($v % 2) continue; $s += 100; } return $s; }
function f4($a) { foreach ($a as $i => $row) { foreach ($row as $j => $c) { if ($c === 'x') return "$i,$j"; } } return 'none'; }
function f5($a) { try { foreach ($a as $v) { if ($v == 2) throw new Exception("e$v"); } } catch (Exception $e) { return $e->getMessage(); } }
function f6() { $a = ['a' => 1, 'b' => 2]; $o = ''; foreach ($a as $k => $v) $o .= "$k$v"; foreach ([] as $v) $o .= 'X'; foreach (new ArrayIterator([7, 8]) as $v) $o .= $v; return $o; }
function f7($a) { $c = 0; foreach ($a as $v) { $c += count($v); } return $c; }
function f8() { $a = [1, 2, 3]; foreach ($a as $v) { unset($a); } return isset($a) ? 'set' : 'unset'; }
for ($r = 0; $r < 3; $r++) {
  echo f1([10, 20, 30]), " | ", f2(), " | ", f3([1, 2, 3, 4, 5]), " | ", f4([[1, 2], [3, 'x']]), " | ", f5([1, 2, 3]), " | ", f6(), " | ", f7([[1], [1, 2], []]), " | ", f8(), "\n";
}
$big = range(1, 100000); $t = 0; foreach ($big as $v) { $t += $v; } echo $t, "\n";
