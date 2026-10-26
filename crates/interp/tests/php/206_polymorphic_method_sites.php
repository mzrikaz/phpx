<?php
class A { function name() { return 'A'; } private function secret() { return 'sa'; } function peek(A $o) { return $o->secret(); } }
class B extends A { function name() { return 'B'; } }
class C extends A {}
class D { function name() { return 'D'; } }
class M { function __call($n, $a) { return "magic:$n"; } }
class Col implements ArrayAccess, IteratorAggregate {
    function __construct(private array $d) {}
    function offsetExists($k): bool { return isset($this->d[$k]); }
    function offsetGet($k): mixed { return $this->d[$k] ?? 'none'; }
    function offsetSet($k, $v): void { if ($k === null) $this->d[] = $v; else $this->d[$k] = $v; }
    function offsetUnset($k): void { unset($this->d[$k]); }
    function getIterator(): Iterator { return new ArrayIterator($this->d); }
}
class It implements Iterator {
    private $i = 0;
    function __construct(private array $v) {}
    function rewind(): void { $this->i = 0; }
    function valid(): bool { return $this->i < count($this->v); }
    function current(): mixed { return $this->v[$this->i]; }
    function key(): mixed { return "k{$this->i}"; }
    function next(): void { $this->i++; }
}
class It2 extends It { function current(): mixed { return strtoupper(parent::current()); } }
function call_all(array $objs) { $out = ''; foreach ($objs as $o) { $out .= $o->name(); } return $out; }
function run() {
    $objs = [new A, new B, new C, new D, new B, new A, new D, new C];
    for ($i = 0; $i < 30; $i++) { $s = call_all($objs); }
    echo $s, "\n";
    $a = new A; echo $a->peek(new B), $a->peek(new C), $a->peek(new A), "\n";
    $mix = [new A, new M, new B, new M];
    foreach ($mix as $o) { echo $o->name(), ' '; } echo "\n";
    $files = [new SplFileInfo('/tmp'), new ArrayIterator([1, 2]), new SplFileInfo('/etc/hostname')];
    for ($i = 0; $i < 3; $i++) { foreach ($files as $f) { echo $f instanceof SplFileInfo ? var_export($f->isDir(), true) : $f->count(), ' '; } }
    echo "\n";
    $c = new Col(['a' => 1]); $c['b'] = 2; $c[] = 3; unset($c['a']);
    echo $c['b'], $c['zz'], isset($c['b']) ? 'Y' : 'N', isset($c['a']) ? 'Y' : 'N', "\n";
    foreach ($c as $k => $v) echo "$k=$v,"; echo "\n";
    foreach ([new It(['x', 'y']), new It2(['p', 'q']), new It(['z'])] as $it) { foreach ($it as $k => $v) echo "$k:$v "; }
    echo "\n";
    try { $d = new D; $d->missing(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
}
run(); run();
