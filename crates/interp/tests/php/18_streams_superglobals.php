<?php
function f() { return "3" . ' ' . count($_GET) . ' ' . "a1"; }
echo f(), " ", "3", " ", "a2", "\n";
$_GET['x'] = 1; function g() { $_GET['y'] = 2; return json_encode($_GET); } echo g(), "\n";
$h = fopen('php://memory', 'w+'); fwrite($h, "line1\nline2\nend"); rewind($h);
while (($l = fgets($h)) !== false) echo "[", rtrim($l), "]"; echo feof($h) ? " eof\n" : " no\n";
var_dump(is_resource($h), get_resource_type($h)); fclose($h); var_dump(is_resource($h), gettype($h));
$t = tempnam(sys_get_temp_dir(), 'px'); $w = fopen($t, 'w'); fputs($w, "abc\ndef\n"); fclose($w);
$r = fopen($t, 'r'); echo fread($r, 2), "|", fgets($r), "|", ftell($r), "|"; fseek($r, -2, SEEK_END); echo json_encode(fread($r, 10)), "\n"; fclose($r); unlink($t);
var_dump(@fopen('/nope/x', 'r'));
$d = opendir(__DIR__); $n = 0; while (($e = readdir($d)) !== false) $n++; closedir($d); echo $n > 2 ? "dir ok\n" : "dir bad\n";
fwrite(STDOUT, "to stdout\n");
echo fnmatch('*.php', 'a.php') ? "y" : "n", fnmatch('[a-c]?', 'bz') ? "y" : "n", "\n";
