<?php
$d = sys_get_temp_dir() . "/phpx_glob_" . getmypid(); foreach (["a/x", "b/x", ".h/x"] as $sub) @mkdir("$d/$sub", 0777, true); foreach (["a/x/m.json", "b/x/m.json", ".h/x/m.json", "a/f.txt", ".dot"] as $f) touch("$d/$f");
$rel = fn($a) => array_map(fn($p) => str_replace($d, 'D', $p), $a);
print_r($rel(glob("$d/*/x/m.json"))); print_r($rel(glob("$d/*"))); print_r($rel(glob("$d/*", GLOB_ONLYDIR | GLOB_MARK))); print_r($rel(glob("$d/{a,b}/*", GLOB_BRACE)));
chdir($d); print_r(glob("*/x/*.json")); print_r(glob("a"));
foreach (["a/x/m.json", "b/x/m.json", ".h/x/m.json", "a/f.txt", ".dot"] as $f) unlink("$d/$f"); foreach (["a/x", "b/x", ".h/x", "a", "b", ".h"] as $sub) rmdir("$d/$sub"); rmdir($d);
[$a, $b] = null; var_dump($a, $b);
@[$c] = 5; var_dump($c);
['k' => $d] = null; var_dump($d);
@[$e] = "str"; var_dump($e);
$x = []; while ([$p, $q] = array_shift($x)) {} echo "ok\n";
function f() { [$a, $b] = null; return $a; } var_dump(f());
