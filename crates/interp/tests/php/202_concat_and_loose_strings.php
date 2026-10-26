<?php
class T { function __construct(public $n) {} function __toString(): string { echo "[conv {$this->n}]"; return "T{$this->n}"; } }
function f($x) { echo "[f $x]"; return $x; }
function run() {
    $a = 'a'; $i = 7; $fl = 1.5; $n = null; $b = true; $o = new T(1);
    for ($k = 0; $k < 3; $k++) {
        echo $a . $i . $fl . $n . $b . false . "\n";
        echo $o . new T(2) . "\n";
        echo f('x') . $o . f('y') . "\n";
        echo 'get' . ucfirst('name') . 'Attribute', "\n";
        $s = $a . $i; $s .= '!'; echo $s, ' ', $a, "\n";
        echo @([1] . 'z'), "\n";
        echo -0.0 . '|' . 1e100 . '|' . PHP_INT_MAX . "\n";
    }
    $cases = [['abc', 'abc'], ['abc', 'ABC'], ['1e3', '1000'], [' 1', '1'], ['1 ', '1'], ['abc', ' abc'], ['0x1A', '26'], ['', '0'], ['null', ''], ['.5', '0.5'], ['a', 'a ']];
    foreach ($cases as [$x, $y]) { var_dump($x == $y, in_array($x, [$y]), in_array($y, ['q', $x], false)); }
    $anon = new class {};
    var_dump(get_class($anon) === get_class($anon), str_starts_with(get_class($anon), 'class@anonymous'), get_class($o), get_class(new ArrayObject([])));
}
run();
