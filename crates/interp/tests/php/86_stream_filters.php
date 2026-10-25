<?php
$h = fopen('php://memory', 'w+');
fwrite($h, "Hello World\nsecond line\n");
rewind($h);
$f = stream_filter_append($h, 'string.toupper', STREAM_FILTER_READ);
var_dump(get_resource_type($f), fgets($h), fread($h, 100));
var_dump(stream_filter_remove($f));
rewind($h);
var_dump(fgets($h));
fclose($h);

$h = fopen('php://temp', 'w+');
stream_filter_append($h, 'string.rot13', STREAM_FILTER_WRITE);
stream_filter_append($h, 'convert.base64-encode', STREAM_FILTER_WRITE);
fwrite($h, "abc");
fwrite($h, "defg");
rewind($h);
var_dump(stream_get_contents($h));

$h = fopen('php://memory', 'w+');
$wf = stream_filter_append($h, 'convert.base64-encode', STREAM_FILTER_WRITE);
fwrite($h, "abcde");
stream_filter_remove($wf);
rewind($h);
var_dump(stream_get_contents($h));
fclose($h);

var_dump(file_get_contents('php://filter/read=string.toupper|string.rot13/resource=data://text/plain,Hello'));
var_dump(file_get_contents('php://filter/read=convert.base64-decode/resource=data://text/plain;base64,' . base64_encode(base64_encode("nested"))));

$tmp = tempnam(sys_get_temp_dir(), 'flt');
file_put_contents('php://filter/write=string.tolower/resource=' . $tmp, "MiXeD CaSe");
var_dump(file_get_contents($tmp));
$h = fopen($tmp, 'w');
stream_filter_append($h, 'zlib.deflate', STREAM_FILTER_WRITE, ['level' => 9]);
fwrite($h, str_repeat("compress me ", 50));
fclose($h);
var_dump(strlen(file_get_contents($tmp)) < 100, gzinflate(file_get_contents($tmp)) === str_repeat("compress me ", 50));
$h = fopen($tmp, 'r');
stream_filter_append($h, 'zlib.inflate', STREAM_FILTER_READ);
var_dump(stream_get_contents($h) === str_repeat("compress me ", 50));
fclose($h);
$h = fopen($tmp, 'w');
stream_filter_append($h, 'zlib.deflate', STREAM_FILTER_WRITE, ['window' => 31]);
fwrite($h, "gzip framed");
fclose($h);
var_dump(gzdecode(file_get_contents($tmp)), bin2hex(substr(file_get_contents($tmp), 0, 4)));

class strtoupper_filter extends php_user_filter
{
    public function filter($in, $out, &$consumed, $closing): int
    {
        while ($bucket = stream_bucket_make_writeable($in)) {
            $bucket->data = strtoupper($bucket->data) . '|';
            $consumed += $bucket->datalen;
            stream_bucket_append($out, $bucket);
        }
        if ($closing) {
            $b = stream_bucket_new($this->stream, "[end:{$this->filtername}:" . json_encode($this->params) . "]");
            stream_bucket_append($out, $b);
        }
        return PSFS_PASS_ON;
    }
    public function onCreate(): bool { echo "create {$this->filtername}\n"; return true; }
    public function onClose(): void { echo "close\n"; }
}
var_dump(stream_filter_register('my.upper', 'strtoupper_filter'), stream_filter_register('my.upper', 'strtoupper_filter'));
var_dump(in_array('my.upper', stream_get_filters()), in_array('string.rot13', stream_get_filters()));
$h = fopen($tmp, 'w');
stream_filter_append($h, 'my.upper', STREAM_FILTER_WRITE, ['x' => 1]);
fwrite($h, "one ");
fwrite($h, "two");
fclose($h);
var_dump(file_get_contents($tmp));
var_dump(@stream_filter_append(fopen('php://memory', 'r'), 'no.such.filter'));
$h = fopen('php://memory', 'w+');
fwrite($h, "4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n");
rewind($h);
stream_filter_append($h, 'dechunk');
var_dump(stream_get_contents($h));
unlink($tmp);
