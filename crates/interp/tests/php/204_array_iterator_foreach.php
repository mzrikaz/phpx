<?php
class Bag implements IteratorAggregate { function __construct(private array $items) {} function getIterator(): Iterator { return new ArrayIterator($this->items); } }
function run() {
    $it = new ArrayIterator(['a' => 1, 'b' => 2, 'c' => 3, 5 => 'five']);
    foreach ($it as $k => $v) { echo "$k=$v "; }
    echo "| valid:", var_export($it->valid(), true), "\n";
    foreach ($it as $k => $v) { if ($k === 'b') break; }
    echo "after break: ", $it->key(), ' ', $it->current(), "\n";
    foreach ($it as $v) { echo $v, ','; foreach ($it as $w) {} }
    echo "\n";
    $bag = new Bag(['x' => 10, 'y' => 20]);
    foreach ($bag as $k => $v) { echo "$k:$v "; }
    foreach ($bag as $k => $v) { echo "$k:$v "; }
    echo "\n", json_encode(iterator_to_array($bag)), ' ', json_encode(iterator_to_array(new ArrayIterator([3, 4]), false)), "\n";
    $e = new ArrayIterator([]); foreach ($e as $v) { echo "never"; } echo "empty ok\n";
    class_exists('Sub') || eval('class Sub extends ArrayIterator { public function current(): mixed { return strtoupper(parent::current()); } }');
    foreach (new Sub(['p' => 'q', 'r' => 's']) as $k => $v) { echo "$k=$v "; }
    echo "\n";
    $gen = (function () use ($bag) { yield from $bag; })();
    foreach ($gen as $k => $v) { echo "$k~$v "; }
    echo "\n";
}
run(); run();
