<?php
$data = ['a' => 1, 'b' => ['c' => 2, 'd' => ['e' => 3, 'f' => 4]], 'g' => 5, 'h' => []];

foreach ([RecursiveIteratorIterator::LEAVES_ONLY, RecursiveIteratorIterator::SELF_FIRST, RecursiveIteratorIterator::CHILD_FIRST] as $mode) {
    $it = new RecursiveIteratorIterator(new RecursiveArrayIterator($data), $mode);
    $out = [];
    foreach ($it as $k => $v) {
        $out[] = str_repeat(' ', $it->getDepth()) . $k . '=' . (is_array($v) ? 'array' : $v);
    }
    echo "mode $mode:\n", implode("\n", $out), "\n";
}

$it = new RecursiveIteratorIterator(new RecursiveArrayIterator($data), RecursiveIteratorIterator::SELF_FIRST);
$it->setMaxDepth(1);
var_dump($it->getMaxDepth());
foreach ($it as $k => $v) echo $k, ' ';
echo "\n";
$it->setMaxDepth();
var_dump($it->getMaxDepth());

class Hooked extends RecursiveIteratorIterator {
    public function beginIteration(): void { echo "[begin]"; }
    public function endIteration(): void { echo "[end]"; }
    public function beginChildren(): void { echo "[in d=", $this->getDepth(), "]"; }
    public function endChildren(): void { echo "[out]"; }
    public function callHasChildren(): bool { return $this->key() !== 'd' && parent::callHasChildren(); }
    public function nextElement(): void { echo "."; }
}
foreach (new Hooked(new RecursiveArrayIterator($data), RecursiveIteratorIterator::SELF_FIRST) as $k => $v) echo $k;
echo "\n";

$it = new RecursiveIteratorIterator(new RecursiveArrayIterator($data));
$it->rewind();
$it->next();
echo get_class($it->getInnerIterator()), ' ', get_class($it->getSubIterator(0)), ' ', var_export($it->getSubIterator(5), true), ' ', $it->getSubIterator()->key(), "\n";

class OddOnly extends FilterIterator {
    public function accept(): bool { return $this->current() % 2 == 1; }
}
echo implode(',', iterator_to_array(new OddOnly(new ArrayIterator(range(1, 10))), false)), "\n";
echo implode(',', iterator_to_array(new CallbackFilterIterator(new ArrayIterator(['x' => 1, 'y' => 2, 'z' => 3]), fn($v, $k) => $k !== 'y'))), "\n";
echo implode(',', iterator_to_array(new LimitIterator(new ArrayIterator(range(1, 10)), 2, 3), false)), "\n";
echo implode(',', iterator_to_array(new LimitIterator(new InfiniteIterator(new ArrayIterator([1, 2, 3])), 0, 7), false)), "\n";
$c = new CachingIterator(new ArrayIterator(['p', 'q', 'r']));
foreach ($c as $v) echo $v, $c->hasNext() ? '+' : '.';
echo "\n";
$n = new NoRewindIterator(new ArrayIterator([1, 2, 3]));
$n->next();
foreach ($n as $v) echo $v;
foreach ($n as $v) echo $v;
echo "\n";
$a = new AppendIterator();
$a->append(new ArrayIterator([1, 2]));
$a->append(new ArrayIterator([3]));
echo implode(',', iterator_to_array($a, false)), "\n";

function gen() { yield 'k1' => 'v1'; yield 'k2' => 'v2'; }
$ii = new IteratorIterator(gen());
foreach ($ii as $k => $v) echo "$k=$v ";
echo "\n";
class Agg implements IteratorAggregate { public function getIterator(): Iterator { return new ArrayIterator([7, 8]); } }
$ii = new IteratorIterator(new Agg());
echo get_class($ii->getInnerIterator()), ' ', implode(',', iterator_to_array($ii)), ' ', $ii->count(), "\n";
var_dump($ii->valid(), $ii->key(), $ii->current());
$ii->rewind();
var_dump($ii->valid(), $ii->key(), $ii->current());

$f = new RecursiveCallbackFilterIterator(new RecursiveArrayIterator($data), fn($v, $k, $i) => $i->hasChildren() || $v > 2);
echo implode(',', iterator_to_array(new RecursiveIteratorIterator($f), false)), "\n";
$p = new RecursiveIteratorIterator(new ParentIterator(new RecursiveArrayIterator($data)), RecursiveIteratorIterator::SELF_FIRST);
foreach ($p as $k => $v) echo $k, ' ';
echo "\n";

class Keys extends IteratorIterator {
    public function current(): mixed { return strtoupper(parent::key()); }
}
echo implode(',', iterator_to_array(new Keys(new ArrayIterator(['m' => 1, 'n' => 2])))), "\n";
foreach ([new IteratorIterator(new ArrayIterator([4])), new RecursiveIteratorIterator(new RecursiveArrayIterator([4]))] as $o) {
    try {
        clone $o;
    } catch (Error $e) {
        echo $e->getMessage(), "\n";
    }
}
