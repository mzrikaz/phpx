<?php
var_dump(hash_pbkdf2("sha256", "password", "salt", 1000, 20), hash_pbkdf2("sha1", "password", "salt", 2), bin2hex(hash_pbkdf2("sha512", "p", "s", 3, 10, true)), hash_pbkdf2("md5", "x", "y", 5, 0, false));
try { hash_pbkdf2("sha256", "p", "s", 0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { hash_pbkdf2("crc32b", "p", "s", 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(bin2hex(hash_hkdf("sha256", "key")), bin2hex(hash_hkdf("sha256", "key", 42, "info", "salt")), strlen(hash_hkdf("sha512", "k", 100)));
try { hash_hkdf("sha256", ""); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { hash_hkdf("sha1", "k", 10000); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$tmp = tempnam(sys_get_temp_dir(), 'hsh');
file_put_contents($tmp, str_repeat("data", 5000));
var_dump(hash_hmac_file("sha256", $tmp, "secret"));
$ctx = hash_init("sha1");
$h = fopen($tmp, 'r');
var_dump(hash_update_stream($ctx, $h, 100), hash_update_stream($ctx, $h));
fclose($h);
var_dump(hash_final($ctx) === sha1(str_repeat("data", 5000)));
$ctx = hash_init("md5");
var_dump(hash_update_file($ctx, $tmp), hash_final($ctx) === md5_file($tmp));
unlink($tmp);
var_dump(bin2hex(@mhash(MHASH_MD5, "abc")), bin2hex(@mhash(MHASH_SHA256, "abc", "key")), @mhash_get_hash_name(MHASH_SHA1), @mhash_get_block_size(MHASH_SHA512), @mhash_count(), bin2hex(@mhash_keygen_s2k(MHASH_SHA1, "pw", "saltsalt", 30)));
var_dump(preg_filter('/\d/', '#', ['a1', 'b', 'c22', 'k' => 'x3']), preg_filter('/x/', 'y', 'abc'), preg_filter(['/a/', '/b/'], ['A', 'B'], 'aabb', 1, $count), $count);
var_dump(filter_var_array(['a' => '12', 'b' => 'x', 'c' => 'me@example.com', 'd' => ['1', '2']], ['a' => FILTER_VALIDATE_INT, 'b' => ['filter' => FILTER_VALIDATE_INT, 'options' => ['default' => 7]], 'c' => FILTER_VALIDATE_EMAIL, 'missing' => FILTER_DEFAULT]));
var_dump(filter_var_array(['x' => '5', 'y' => 'abc'], FILTER_VALIDATE_INT), filter_var_array(['x' => '1'], ['x' => FILTER_DEFAULT, 'z' => FILTER_DEFAULT], false));
var_dump(filter_input_array(INPUT_GET));
var_dump(spl_autoload_extensions(), spl_autoload_extensions('.php'), spl_autoload_extensions());
spl_autoload_register(function ($c) { echo "autoload $c\n"; });
spl_autoload_call('Nope\Missing');
var_dump(count(spl_classes()) > 40, spl_classes()['ArrayObject']);
