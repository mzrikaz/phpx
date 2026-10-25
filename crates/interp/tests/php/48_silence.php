<?php
function f() { throw new Exception("x"); }
function g($a) { return $a[5]; }
$base = error_reporting();
try { @f(); } catch (Exception $e) { echo "caught ", error_reporting() === $base ? "restored" : "leaked", "\n"; }
$a = [];
var_dump(@$a[3]);
try { $x = @g([]) + @f(); } catch (Exception $e) { echo "c2 ", error_reporting() === $base ? "restored" : "leaked", "\n"; }
function h() { try { return @(fn() => throw new RuntimeException("y"))(); } finally { echo "fin ", error_reporting() === 0 ? "silenced" : "normal", "\n"; } }
try { h(); } catch (RuntimeException $e) { echo "c3\n"; }
var_dump(@file_get_contents("/nonexistent/phpx"));
echo "done\n";
