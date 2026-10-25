<?php
$k = str_repeat("k", 32); $iv = str_repeat("i", 16);
foreach (['aes-128-cbc','AES-256-CBC','aes-192-ecb','aes-256-ctr','aes-128-cfb','aes-128-cfb8','aes-256-ofb','aes-256-gcm','aes-128-gcm','aes-128-cfb1','nope'] as $c) {
  var_dump($c, openssl_cipher_iv_length($c), openssl_cipher_key_length($c));
}
foreach (["", "a", str_repeat("x", 16), str_repeat("y", 33)] as $d) {
  foreach (['aes-128-cbc','aes-256-cbc','aes-192-ecb','aes-256-ctr','aes-128-cfb','aes-128-cfb8','aes-256-ofb'] as $c) {
    $e = openssl_encrypt($d, $c, $k, 0, substr($iv, 0, openssl_cipher_iv_length($c)));
    var_dump($e, openssl_decrypt($e, $c, $k, 0, substr($iv, 0, openssl_cipher_iv_length($c))));
    $e = openssl_encrypt($d, $c, "short", OPENSSL_RAW_DATA, $iv);
    var_dump(bin2hex($e));
  }
  foreach ([12, 16, 8] as $ivl) foreach ([16, 12, 4] as $tl) {
    $tag = null;
    $e = openssl_encrypt($d, 'aes-256-gcm', $k, 0, substr(str_repeat("n", 20), 0, $ivl), $tag, "aad", $tl);
    var_dump($e, bin2hex($tag), openssl_decrypt($e, 'aes-256-gcm', $k, 0, substr(str_repeat("n", 20), 0, $ivl), $tag, "aad"));
    var_dump(openssl_decrypt($e, 'aes-256-gcm', $k, 0, substr(str_repeat("n", 20), 0, $ivl), "badtagbadtagbadt", "aad"));
  }
}
var_dump(openssl_encrypt("x", "aes-128-cbc", $k, 0, "short"));
var_dump(openssl_encrypt("x", "aes-128-cbc", $k, 0, str_repeat("l", 20)));
var_dump(openssl_encrypt("x", "aes-128-cbc", $k));
var_dump(openssl_encrypt("x", "aes-128-cbc", $k, OPENSSL_ZERO_PADDING, $iv));
var_dump(bin2hex(openssl_encrypt(str_repeat("x",16), "aes-128-cbc", $k, OPENSSL_ZERO_PADDING|OPENSSL_RAW_DATA, $iv)));
var_dump(openssl_decrypt("!!!", "aes-128-cbc", $k, 0, $iv));
var_dump(openssl_decrypt("AAAAAAAAAAAAAAAAAAAAAA==", "aes-128-cbc", $k, 0, $iv));
var_dump(openssl_decrypt("abc", "aes-256-gcm", $k, 0, "123456789012"));
var_dump(openssl_encrypt("x", "nope", $k, 0, $iv));
var_dump(openssl_decrypt("x", "nope", $k, 0, $iv));
var_dump(openssl_encrypt("x", "aes-256-gcm", $k, 0, ""));
$t=null; var_dump(openssl_encrypt("x", "aes-256-gcm", $k, 0, "123456789012", $t, "", 3));
$t=null; var_dump(openssl_encrypt("x", "aes-256-gcm", $k, 0, "123456789012", $t, "", 17));
var_dump(openssl_digest("abc", "sha256"), openssl_digest("abc", "SHA1", true) === sha1("abc", true), openssl_digest("abc", "md5"), openssl_digest("abc", "nope"));
var_dump(bin2hex(openssl_pbkdf2("pw", "salt", 20, 1000)), bin2hex(openssl_pbkdf2("pw", "salt", 32, 10, "sha256")), openssl_pbkdf2("pw", "salt", 32, 10, "nope"));
$m = openssl_get_cipher_methods(); var_dump(in_array("aes-256-cbc", $m), in_array("AES-256-CBC", $m), count($m) > 10);
var_dump(OPENSSL_RAW_DATA, OPENSSL_ZERO_PADDING, OPENSSL_DONT_ZERO_PAD_KEY);
while ($e = openssl_error_string()) {}
var_dump(openssl_error_string());
