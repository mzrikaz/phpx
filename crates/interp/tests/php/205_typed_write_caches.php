<?php
interface Shape {}
class Sq implements Shape {}
class Ci implements Shape {}
class Box {
    public string $s = '';
    public float $f = 0.0;
    public ?Shape $shape = null;
    public array $opts = [];
    public readonly array $ro;
    private array $priv = [];
    public $plain = [];
    function __construct() { $this->ro = ['a' => 1]; }
    function setS($v) { $this->s = $v; return $this; }
    function setF($v) { $this->f = $v; }
    function setShape($v) { $this->shape = $v; }
    function opt($k, $v) { $this->opts[$k] = $v; }
    function push($v) { $this->opts[] = $v; }
    function priv($k) { $this->priv[$k] = 1; return count($this->priv); }
    function me(): static { return $this; }
    function take(Shape $s): Shape { return $s; }
}
class SubBox extends Box {}
function run() {
    $b = new Box;
    for ($i = 0; $i < 3; $i++) {
        $b->setS('x'); $b->setS(5); var_dump($b->s);
        $b->setF(1.5); $b->setF(2); var_dump($b->f);
        $b->setShape(new Sq); $b->setShape(null); $b->setShape(new Ci); echo get_class($b->shape), "\n";
        try { $b->setShape(new ArrayObject); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
        try { $b->setS([]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
        $b->opt('k', $i); $b->opt(7, 'seven'); $b->opt('08', 'str'); $b->push('p'); echo json_encode($b->opts), "\n";
        echo $b->priv("p$i"), "\n";
        try { $b->ro['b'] = 2; } catch (Error $e) { echo $e->getMessage(), "\n"; }
        $r = &$b->plain; $b->plain['z'] = 1; echo json_encode($r), "\n"; unset($r);
        echo get_class((new SubBox)->me()), get_class($b->me()), "\n";
        echo get_class($b->take(new Sq)), get_class($b->take(new Ci)), "\n";
        try { $b->take(new ArrayObject); } catch (TypeError $e) { echo substr($e->getMessage(), 0, 60), "\n"; }
    }
    var_dump(rawurldecode('a%+5b%2Fc%zz%4%41'), urldecode('x+y%41%'), rtrim('/a/b//', '/'), ltrim('..x', '.'), trim("\t q \n"), rtrim('abc', 'a..c'), trim('xx', 'x'), trim('ab', ''));
}
run();
