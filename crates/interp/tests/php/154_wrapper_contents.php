<?php
class MemW {
    static $data = [];
    public $context;
    private $p; private $pos = 0;
    function stream_open($path, $mode, $o, &$op) { $this->p = $path; if (str_contains($mode, 'w')) self::$data[$path] = ''; $this->pos = 0; return true; }
    function stream_read($n) { $r = substr(self::$data[$this->p] ?? '', $this->pos, $n); $this->pos += strlen($r); return $r; }
    function stream_write($d) { self::$data[$this->p] .= $d; return strlen($d); }
    function stream_eof() { return $this->pos >= strlen(self::$data[$this->p] ?? ''); }
    function stream_stat() { return []; }
    function url_stat($p, $f) { return isset(self::$data[$p]) ? ['size' => strlen(self::$data[$p])] : false; }
}
stream_wrapper_register('mem', 'MemW');
var_dump(file_put_contents('mem://a', 'hello world'));
var_dump(file_get_contents('mem://a'), file_get_contents('mem://a', false, null, 6, 3), file_exists('mem://a'), file_exists('mem://b'));
var_dump(file_get_contents('data:text/plain,hi%20there'), file_get_contents('data://text/plain;base64,SGk='));
