<?php foreach ([fn() => array_rand([]), fn() => substr_count("a", ""), fn() => explode("", "a"), fn() => str_split("a", 0), fn() => range(1, 2, 0), fn() => array_fill(0, -1, 1), fn() => str_pad("a", 5, "")] as $f) { try { $f(); } catch (ValueError $e) { echo $e->getMessage(), "\n"; } }

$c = [[1, 1.0], [1.0, 1], [1, 1.0, 1], [1.0, 1, 1.0], ["10", "1e1"], ["1e1", "10"], ["a", "a"], [0, "a"], ["a", 0], [[1], [1]], [null, false], [false, null], [1, true], [true, 1]];
foreach ($c as $args) echo var_export(min(...$args), true), " ", var_export(max(...$args), true), " | ";
echo "\n"; var_export(min([1, 1.0])); echo " "; var_export(max([1.0, 1]));
