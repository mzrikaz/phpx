<?php
// realpath() results are cached, and unlink/rename/rmdir/clearstatcache drop them.
$d = sys_get_temp_dir() . "/rpt" . getmypid(); @mkdir($d); $f = "$d/a.txt"; file_put_contents($f, "x");
var_dump(realpath($f) === $f);
unlink($f); var_dump(realpath($f));
file_put_contents($f, "x"); var_dump(realpath($f) === $f);
rename($f, "$d/b.txt"); var_dump(realpath($f), realpath("$d/b.txt") === "$d/b.txt");
unlink("$d/b.txt"); rmdir($d); var_dump(realpath($d));
$f2 = sys_get_temp_dir() . "/rpt2_" . getmypid();
file_put_contents($f2, "x"); var_dump(realpath($f2) === $f2);
clearstatcache(true, $f2); var_dump(realpath($f2) === $f2);
unlink($f2); var_dump(realpath($f2));
