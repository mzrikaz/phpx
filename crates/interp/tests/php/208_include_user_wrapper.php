<?php
class CodeStream {
    public $context;
    private string $code = '';
    private int $pos = 0;
    public static array $opened = [];
    function stream_open($path, $mode, $options, &$opened_path) {
        self::$opened[] = $mode;
        $this->code = '<?php ' . substr($path, strlen('code://'));
        return !str_contains($path, 'missing');
    }
    function stream_read($count) { $r = substr($this->code, $this->pos, $count); $this->pos += strlen($r); return $r; }
    function stream_eof() { return $this->pos >= strlen($this->code); }
    function stream_stat() { return []; }
    function stream_set_option($o, $a, $b) { return false; }
}
stream_wrapper_register('code', 'CodeStream');
function run() {
    $x = 20;
    $f = include 'code://return function ($y) use ($x) { return $x + $y; };';
    echo $f(1), "\n";
    echo include 'code://$x++; return "x is $x";', ' ', $x, "\n";
    $long = 'code://return "' . str_repeat('ab', 6000) . '";';
    echo strlen(include $long), "\n";
    var_dump(include_once 'code://return 7;', include_once 'code://return 7;');
    var_dump(@include 'code://missing');
    echo json_encode(array_unique(CodeStream::$opened)), "\n";
}
run();
