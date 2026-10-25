<?php
function h($no, $str, $file, $line) { echo "H: $no $str ", basename($file), ":$line er=", error_reporting(), "\n"; return true; }
var_dump(set_error_handler('h'));
$a = []; echo $a['x'] ?? 'dflt', "\n";
echo $a['y'], "|\n";
echo @$a['z'], "|\n";
echo $undefined, "|\n";
trigger_error("custom", E_USER_WARNING);
var_dump(set_error_handler(function ($n, $s) { throw new ErrorException($s, 0, $n); }));
try { $x = [] ; $y = $x['nope']; echo "after\n"; } catch (ErrorException $e) { echo "caught: ", $e->getMessage(), "\n"; }
try { strpos(); } catch (ArgumentCountError $e) { echo "ace\n"; }
try { $r = file_get_contents('/nonexistent/file'); echo "continued\n"; } catch (ErrorException $e) { echo "caught2: ", $e->getMessage(), "\n"; }
try { $v = @file_get_contents('/nonexistent/file2'); echo "silenced continues\n"; } catch (ErrorException $e) { echo "caught3\n"; }
restore_error_handler();
set_error_handler(fn($n, $s) => false, E_NOTICE);
echo $a['w'], "|\n";
restore_error_handler(); restore_error_handler();
@strlen($zz);
var_dump(error_get_last());
error_clear_last(); var_dump(error_get_last());
var_dump(error_reporting(E_ALL & ~E_WARNING));
echo $a['q'], "|\n";
error_reporting(E_ALL);
set_exception_handler(function ($e) { echo "EH: ", $e->getMessage(), "\n"; });
throw new RuntimeException("boom");
