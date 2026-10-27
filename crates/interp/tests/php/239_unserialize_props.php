<?php
class Base {
    private $secret = 'base';
    protected $prot = 'p';
    public $pub = 1;
    public ?int $typed;
}
class Kid extends Base {
    private $secret = 'kid';
    public $extra = [1, 2];
    public function show() { return get_object_vars($this); }
}
class Woke {
    public $a = 1;
    public $log = [];
    public function __wakeup() { $this->log[] = 'woke:' . $this->a; }
}
class Custom {
    public $got;
    public function __serialize(): array { return ['x' => 1, 5 => 'five']; }
    public function __unserialize(array $d): void { $this->got = $d; }
}
$k = new Kid; $k->pub = 'changed'; $k->typed = 7; $k->dyn = 'dynamic';
$w = new Woke; $w->a = 9;
$all = [$k, $k, $w, new Custom, 'tail' => $w];
for ($i = 0; $i < 2; $i++) {
    $s = serialize($all);
    $u = unserialize($s);
    var_dump($u[0] === $u[1], $u['tail'] === $u[2]);
    var_dump($u[0]->show(), $u[2], $u[3]->got);
    print_r((array) $u[0]);
}
// Reordered and unknown properties, int keys, a missing class.
$s = 'O:4:"Woke":4:{s:3:"log";a:0:{}i:7;s:3:"int";s:1:"a";i:3;s:3:"new";b:1;}';
var_dump(unserialize($s));
var_dump(unserialize('O:7:"Missing":1:{s:1:"q";i:1;}'));
var_dump(unserialize('O:4:"Woke":1:{s:1:"a";i:1;}', ['allowed_classes' => false]));
$t = unserialize('O:3:"Kid":1:{s:5:"typed";N;}');
var_dump($t->typed);
