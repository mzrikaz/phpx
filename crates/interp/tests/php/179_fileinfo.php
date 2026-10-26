<?php
$samples = [
    "txt" => "hello world\n", "json" => "{\"a\":1}\n", "csv" => "a,b\n1,2\n", "php" => "<?php echo 1;\n",
    "html" => "<!DOCTYPE html><html><body>x</body></html>\n", "xml" => "<?xml version=\"1.0\"?><r/>\n",
    "png" => "\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\1\0\0\0\1\x08\x06\0\0\0\x1f\x15\xc4\x89", "gif" => "GIF89a\1\0\1\0",
    "pdf" => "%PDF-1.4\n%\xe2\xe3\n", "gz" => "\x1f\x8b\x08\0\0\0\0\0", "utf8" => "h\xc3\xa9llo\n", "empty" => "",
    "bin" => "\0\1\2\3\xff", "sh" => "#!/bin/sh\necho\n", "svg" => "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>\n",
    "webp" => "RIFF\x24\0\0\0WEBPVP8 ", "nolf" => "no newline", "crlf" => "a\r\nb\r\n", "latin1" => "caf\xe9\n",
];
$t = new finfo(FILEINFO_MIME_TYPE); $m = new finfo(FILEINFO_MIME); $d = new finfo();
foreach ($samples as $k => $v) echo $k, " | ", $t->buffer($v), " | ", $m->buffer($v), " | ", $d->buffer($v), "\n";
$f = tempnam(sys_get_temp_dir(), 'fi'); file_put_contents($f, $samples['png']);
echo mime_content_type($f), " ", finfo_file(finfo_open(FILEINFO_MIME_TYPE), $f), " ", (new finfo(FILEINFO_EXTENSION))->file($f), "\n";
unlink($f);
var_dump(@$d->file('/nonexistent/file'));
