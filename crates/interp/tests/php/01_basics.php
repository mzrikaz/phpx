<?php
// Scalars, operators, conversions.
$i = PHP_INT_MAX;
var_dump($i + 1, -$i - 2, $i * 2, 7 / 2, 6 / 3, 7 % 3, -7 % 3, 2 ** 10, 2 ** 64, 2 ** -1, intdiv(7, 2));
var_dump(0.1 + 0.2, 1e15, 1e14, 0.00001, -0.0, 1.5e-7, 100.0, 1/3);
var_dump("5" + 3, "1.5" + 1, "1e3" * 1, " 42 " + 0, 10 <=> 2, "abc" <=> "abd");
var_dump((int) "12abc", (int) 3.99, (int) -3.99, (float) "1e2", (string) 1.0, (bool) "0", (bool) "0.0", (bool) [], (int) true);
var_dump(0 == "a", "1" == "01", "10" == "1e1", 100 == "1e2", null == false, [] == false, "abc" == 0, null == 0, "" == null);
var_dump(1 <=> 2.5, [1, 2] == [1, 2], [1, 2] === [2 => 1], "Z" < "a", null < -1);
var_dump(7 & 3, 7 | 8, 7 ^ 2, ~5, 1 << 3, -16 >> 2, "a" . 1 . 2.5 . true . null);
$x = 5;
$x += 2; $x -= 1; $x *= 3; $x /= 2; $x **= 2;
var_dump($x);
$s = "a"; $s .= "b"; $s .= 1;
var_dump($s);
$n = null; $n ??= "default"; var_dump($n);
$z = "z"; $z++; var_dump($z);
$az = "Az"; $az++; var_dump($az);
$k = null; $k++; var_dump($k);
$m = null; $m--; var_dump($m);
var_dump(PHP_EOL, PHP_INT_SIZE, M_PI);
var_dump(true ? "yes" : "no", null ?: "elvis", 0 ?: false ?: "last");
var_dump(is_numeric("12"), is_numeric("12.5e3"), is_numeric("abc"), is_numeric(" 5"), is_numeric("5 "));
var_dump(gettype(1), gettype(1.0), gettype("s"), gettype(null), gettype([]), get_debug_type(1.5));
var_dump(intval("0x1A", 16), intval("012", 0), intval("101", 2), octdec("777"), hexdec("ff"), bindec("1010"));
var_dump(round(2.5), round(-2.5), round(1.955, 2), round(1234.5678, -2), floor(-1.5), ceil(-1.5), abs(-5), abs(-5.5));
var_dump(max(1, 2.5, "3"), min([4, 2, 8]), max("apple", "banana"), array_sum([1, 2.5, "3"]), array_product([2, "3", 4]));
