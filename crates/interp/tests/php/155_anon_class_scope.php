<?php
abstract class Base { abstract protected function doIt(): void; public function __toString(): string { return 'b'; } }
function make($x) {
    return new class ($x, 'y') extends Base2 {
        private $a; private $b;
        public function __construct($a, $b) { $this->a = $a; $this->b = $b; }
        protected function doIt(): void {}
        public function get() { return $this->a . $this->b; }
    };
}
echo make('x')->get(), "\n";
$o = new class { private $p = 1; function set() { $this->p = 2; return $this->p; } };
echo $o->set(), "\n";
abstract class Base2 { }
$o = new class (1) extends Base2 {
    private $a;
    public function __construct($a) { var_dump(self::class === static::class, get_class($this) === self::class); }
};
$p = new class { public function __construct() { var_dump(self::class === static::class); } };
class B {} interface I {}
$a = new class extends B {}; $b = new class implements I {}; $c = new class {};
var_dump($a, $b instanceof I); echo strtok(get_class($a), "\0"), " ", strtok(get_class($b), "\0"), " ", strtok(get_class($c), "\0"), "\n";
