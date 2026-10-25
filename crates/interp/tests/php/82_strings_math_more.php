<?php
var_dump(strrpos("hello hello", "l"), strrpos("hello hello", "l", 4), strrpos("hello hello", "l", -3), strrpos("hello hello", "l", -9), strrpos("abc", "c", 3));
var_dump(strripos("HeLLo hello", "LL"), strripos("HeLLo hello", "ll", -7), strripos("abc", "x"));
try { strrpos("abc", "a", 5); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { strripos("abc", "a", -5); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(strchr("user@example.com", "@"), strchr("user@example.com", "@", true));
var_dump(quotemeta("1+1=2? [yes] (no) $5 ^.*\\"), quotemeta(""));
var_dump(count_chars("abacab", 1), count_chars("abacab", 3), strlen(count_chars("abc", 4)), count(count_chars("x", 0)), count(count_chars("xyz", 2)));
var_dump(str_rot13("Hello, World!"));
var_dump(strlen(str_shuffle("abcdef")), count_chars(str_shuffle("aabbc"), 3));
foreach (["a", "z", "Az", "zz", "Zz", "9", "a9", "Zz9", "zZ"] as $s) echo $s, "=>", str_increment($s), " ";
echo "\n";
foreach (["b", "aa", "Ba", "10", "1", "A0", "Aa", "zA", "100"] as $s) echo $s, "=>", str_decrement($s), " ";
echo "\n";
foreach (["a", "0", "01", "", "a-b"] as $s) { try { str_decrement($s); } catch (ValueError $e) { echo $e->getMessage(), "\n"; } }
foreach (["Thompson", "knight", "Christmas", "school", "physician", "Xavier", "wright", "gnome", "aero", "judge", "tough", "science", "bitch", "laugh", "whistle", "rough", "agnes", "Thumb", "ignored", "ciao", "", "123"] as $w) echo $w, ":", metaphone($w), " ";
echo "\n", metaphone("Thompson", 2), "\n";
var_dump(strcoll("a", "b") < 0, strcoll("b", "a") > 0, strcoll("a", "a"));
$u = convert_uuencode("test\ntext text text\r\n");
var_dump($u, convert_uudecode($u));
$long = str_repeat("The quick brown fox jumps over the lazy dog. ", 3);
var_dump(convert_uudecode(convert_uuencode($long)) === $long, convert_uuencode(""));
var_dump(quoted_printable_decode("=48=65llo=20W=6Frld=\r\nnext line=\nx = y=3D=xx=0A"));
printf("%.10f %.10f %.10f %.10f %.10f %.10f\n", asin(0.5), acos(0.5), sinh(1), cosh(1), tanh(1), asinh(1));
printf("%.10f %.10f %.10f %.10f %.10f\n", acosh(2), atanh(0.5), expm1(1e-5), log1p(1e-5), hypot(3, 4));
var_dump(deg2rad(180), rad2deg(M_PI), fdiv(1, 0), fdiv(-1, 0), is_nan(fdiv(0, 0)), fpow(2, 10), fpow(-8, 1/3));
$t = gettimeofday(); var_dump(array_keys($t), is_float(gettimeofday(true)));
