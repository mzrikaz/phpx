<?php
$f = tempnam(sys_get_temp_dir(), 'gz');
$text = "line one\nline two\n" . str_repeat("payload ", 200) . "\nlast";
$h = gzopen($f, 'wb9');
var_dump(gzwrite($h, $text), gzclose($h));
var_dump(gzdecode(file_get_contents($f)) === $text, bin2hex(substr(file_get_contents($f), 0, 3)));
$h = gzopen($f, 'rb');
var_dump(gzgets($h), gzread($h, 4), gzgetc($h), gztell($h), gzeof($h));
var_dump(gzseek($h, 0), gzgets($h), gzrewind($h), strlen(gzread($h, 100000)) === strlen($text), gzeof($h));
gzclose($h);
var_dump(count(gzfile($f)), gzfile($f)[1]);
ob_start(); $n = readgzfile($f); $out = ob_get_clean();
var_dump($n === strlen($text), $out === $text);
var_dump(file_get_contents('compress.zlib://' . $f) === $text);
file_put_contents($f, "not compressed\n");
var_dump(gzfile($f));
unlink($f);
var_dump(@gzopen('/nonexistent/x.gz', 'r'));
foreach ([ZLIB_ENCODING_RAW, ZLIB_ENCODING_DEFLATE, ZLIB_ENCODING_GZIP] as $enc) {
    $d = deflate_init($enc, ['level' => 6]);
    $c = deflate_add($d, "Hello ", ZLIB_NO_FLUSH) . deflate_add($d, "incremental ", ZLIB_SYNC_FLUSH) . deflate_add($d, "world", ZLIB_FINISH);
    $plain = match ($enc) { ZLIB_ENCODING_RAW => gzinflate($c), ZLIB_ENCODING_DEFLATE => gzuncompress($c), ZLIB_ENCODING_GZIP => gzdecode($c) };
    $i = inflate_init($enc);
    $back = '';
    foreach (str_split($c, 5) as $chunk) $back .= inflate_add($i, $chunk, ZLIB_SYNC_FLUSH);
    var_dump(get_class($d), $plain, $back, inflate_get_status($i), inflate_get_read_len($i) === strlen($c));
}
try { deflate_init(99); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(zlib_get_coding_type(), ob_gzhandler("x", PHP_OUTPUT_HANDLER_START));
