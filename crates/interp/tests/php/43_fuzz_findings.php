<?php
// Inputs the fuzzers found crashing phpx.
var_dump(@unpack("x5/H2", "ab"), @unpack("@9/C", "ab"), unpack("X3/C", "ab"), @unpack("a18446744073709551615", "ab"), unpack("H*", "ab"), unpack("x/h3", "abc"));
var_dump(intval("12", 1), intval("12", 37), intval("12", -5), intval("0x1A", 0), intval("012", 0), intval("z", 36), intval(42, 1));
echo base_convert("ff", 16, 2), " ", base_convert("zz", 36, 10), "\n";
foreach ([[1, 10], [10, 37]] as [$f, $t]) { try { base_convert("1", $f, $t); } catch (ValueError $e) { echo $e->getMessage(), "\n"; } }
echo number_format(1234.5, -2), " ", strlen(number_format(1.5, 100000)), " ", number_format(0.1, 30), "\n";
eval('echo "unterminated\\');
