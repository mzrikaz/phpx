<?php
class MemWrapper
{
    public static array $files = [];
    public static array $dirs = ['mem://' => true];
    public $context;
    private string $path;
    private int $pos = 0;

    public function stream_open($path, $mode, $options, &$opened)
    {
        $this->path = $path;
        if (str_contains($mode, 'w')) {
            self::$files[$path] = '';
        } elseif (!isset(self::$files[$path])) {
            if (str_contains($mode, 'a') || str_contains($mode, 'c') || str_contains($mode, 'x')) {
                self::$files[$path] = '';
            } else {
                return false;
            }
        }
        if (str_contains($mode, 'a')) {
            $this->pos = strlen(self::$files[$path]);
        }
        return true;
    }
    public function stream_read($count)
    {
        $r = substr(self::$files[$this->path], $this->pos, $count);
        $this->pos += strlen($r);
        return $r;
    }
    public function stream_write($data)
    {
        $f = &self::$files[$this->path];
        $f = substr($f, 0, $this->pos) . $data . substr($f, $this->pos + strlen($data));
        $this->pos += strlen($data);
        return strlen($data);
    }
    public function stream_eof() { return $this->pos >= strlen(self::$files[$this->path]); }
    public function stream_tell() { return $this->pos; }
    public function stream_seek($offset, $whence)
    {
        $len = strlen(self::$files[$this->path]);
        $p = match ($whence) { SEEK_SET => $offset, SEEK_CUR => $this->pos + $offset, SEEK_END => $len + $offset };
        if ($p < 0) return false;
        $this->pos = $p;
        return true;
    }
    public function stream_stat() { return ['size' => strlen(self::$files[$this->path]), 'mode' => 0100644]; }
    public function stream_close() { echo "close {$this->path}\n"; }
    public function stream_flush() { return true; }
    public function url_stat($path, $flags)
    {
        if (isset(self::$files[$path])) return ['mode' => 0100644, 'size' => strlen(self::$files[$path]), 'mtime' => 1700000000];
        if (isset(self::$dirs[rtrim($path, '/')]) || isset(self::$dirs[$path])) return ['mode' => 040755, 'size' => 0];
        return false;
    }
    public function unlink($path) { if (!isset(self::$files[$path])) return false; unset(self::$files[$path]); return true; }
    public function rename($from, $to) { self::$files[$to] = self::$files[$from]; unset(self::$files[$from]); return true; }
    public function mkdir($path, $mode, $options) { self::$dirs[$path] = true; return true; }
    public function rmdir($path, $options) { unset(self::$dirs[$path]); return true; }
    private array $listing = [];
    public function dir_opendir($path, $options)
    {
        $prefix = rtrim($path, '/') . '/';
        $this->listing = [];
        foreach (array_merge(array_keys(self::$files), array_keys(self::$dirs)) as $p) {
            if (str_starts_with($p, $prefix) && !str_contains(substr($p, strlen($prefix)), '/') && $p !== $prefix) $this->listing[] = substr($p, strlen($prefix));
        }
        return true;
    }
    public function dir_readdir() { return array_shift($this->listing) ?? false; }
    public function dir_closedir() { return true; }
    public function stream_metadata($path, $option, $value)
    {
        if ($option === STREAM_META_TOUCH && !isset(self::$files[$path])) self::$files[$path] = '';
        return true;
    }
}

var_dump(stream_wrapper_register('mem', 'MemWrapper'));
var_dump(@stream_wrapper_register('mem', 'MemWrapper'));
var_dump(in_array('mem', stream_get_wrappers()));
var_dump(file_put_contents('mem://a.txt', "line1\nline2\nline3\n"));
var_dump(file_get_contents('mem://a.txt'));
var_dump(file_exists('mem://a.txt'), is_file('mem://a.txt'), is_dir('mem://a.txt'), file_exists('mem://nope'), filesize('mem://a.txt'), filemtime('mem://a.txt'));
$h = fopen('mem://a.txt', 'r+');
var_dump(fgets($h), fread($h, 3), ftell($h), feof($h));
fseek($h, 0, SEEK_END);
fwrite($h, "line4\n");
rewind($h);
var_dump(stream_get_contents($h), feof($h), fstat($h)['size']);
$m = stream_get_meta_data($h);
var_dump($m['wrapper_type'], $m['stream_type'], get_class($m['wrapper_data']));
fclose($h);
var_dump(file('mem://a.txt', FILE_IGNORE_NEW_LINES));
var_dump(@fopen('mem://missing', 'r'));
var_dump(mkdir('mem://dir'), is_dir('mem://dir'), touch('mem://dir/x'), file_exists('mem://dir/x'));
file_put_contents('mem://dir/y', 'yy');
var_dump(scandir('mem://dir'));
$d = opendir('mem://dir');
$seen = [];
while (($e = readdir($d)) !== false) $seen[] = $e;
closedir($d);
sort($seen);
var_dump($seen);
var_dump(rename('mem://dir/y', 'mem://dir/z'), file_get_contents('mem://dir/z'), copy('mem://dir/z', 'mem://dir/w'), file_get_contents('mem://dir/w'));
var_dump(unlink('mem://dir/z'), file_exists('mem://dir/z'), rmdir('mem://dir'));
var_dump(stream_wrapper_unregister('mem'), @file_get_contents('mem://a.txt'));
$tmp = tempnam(sys_get_temp_dir(), 'phpx');
file_put_contents($tmp, 'plain');
var_dump(stream_wrapper_unregister('file'), @file_get_contents($tmp), stream_wrapper_restore('file'), file_get_contents($tmp));
unlink($tmp);
