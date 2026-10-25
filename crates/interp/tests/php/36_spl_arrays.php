<?php
function show($label, $v) { echo "== $label\n"; var_dump($v); }
$ao = new ArrayObject(['b' => 2, 'a' => 1, 3 => 'x']);
show('ao', $ao);
print_r($ao); echo "\n";
echo count($ao), " ", $ao['a'], " ", isset($ao['zz']) ? 'y' : 'n', " ", isset($ao['b']) ? 'y' : 'n', "\n";
$ao['c'] = 3; $ao[] = 'pushed'; unset($ao['b']);
foreach ($ao as $k => $v) echo "$k=$v ";
echo "\n";
var_dump($ao->getArrayCopy(), iterator_to_array($ao), (array)$ao, json_encode($ao), get_object_vars($ao));
$ao->ksort(); echo implode(",", array_keys($ao->getArrayCopy())), "\n";
$ao->asort(); echo implode(",", $ao->getArrayCopy()), "\n";
$ao->uasort(fn($x, $y) => strcmp((string)$y, (string)$x)); echo implode(",", $ao->getArrayCopy()), "\n";
$old = $ao->exchangeArray([9, 8]); echo count($old), " ", count($ao), "\n";
$ao->append(7); echo implode(",", $ao->getArrayCopy()), " ", get_class($ao->getIterator()), " ", $ao->getIteratorClass(), "\n";
$s = serialize($ao); echo $s, "\n"; $u = unserialize($s); var_dump($u == $ao, $u->getArrayCopy());
$it = new ArrayIterator([10, 20, 30, 40]);
show('it', $it);
foreach ($it as $k => $v) { echo "$k:$v "; } echo "\n";
$it->rewind(); $it->seek(2); echo $it->key(), "=", $it->current(), "\n";
try { $it->seek(10); } catch (OutOfBoundsException $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$it->rewind(); while ($it->valid()) { echo $it->current(), ";"; $it->next(); } echo "\n";
var_dump($it->current(), $it->key(), count($it));
$r = new RecursiveIteratorIterator(new RecursiveArrayIterator([1, [2, 3, [4]], 5]));
foreach ($r as $k => $v) echo "$k:$v(d{$r->getDepth()}) "; echo "\n";
$r = new RecursiveIteratorIterator(new RecursiveArrayIterator(['a' => 1, 'b' => ['c' => 2]]), RecursiveIteratorIterator::SELF_FIRST);
foreach ($r as $k => $v) echo $k, "=", is_array($v) ? 'arr' : $v, " "; echo "\n";
echo implode(",", iterator_to_array(new LimitIterator(new ArrayIterator(range(1, 10)), 2, 3), false)), "\n";
echo implode(",", iterator_to_array(new CallbackFilterIterator(new ArrayIterator(range(1, 10)), fn($v) => $v % 3 == 0), false)), "\n";
$ap = new AppendIterator(); $ap->append(new ArrayIterator([1, 2])); $ap->append(new ArrayIterator([3]));
echo implode(",", iterator_to_array($ap, false)), "\n";
$c = new CachingIterator(new ArrayIterator(['x', 'y', 'z']));
foreach ($c as $v) { echo $v, $c->hasNext() ? "," : "\n"; }
echo implode(",", iterator_to_array(new IteratorIterator(new ArrayIterator([5, 6])))), "\n";
$inf = new InfiniteIterator(new ArrayIterator([1, 2])); $n = 0; foreach ($inf as $v) { echo $v; if (++$n >= 5) break; } echo "\n";
echo iterator_count(new ArrayIterator([1, 2, 3])), " ", count(iterator_to_array(new EmptyIterator())), "\n";
function gen() { yield 1; yield 2; }
echo implode(",", iterator_to_array(new NoRewindIterator(gen()))), "\n";
class Evens extends FilterIterator { public function accept(): bool { return $this->current() % 2 == 0; } }
echo implode(",", iterator_to_array(new Evens(new ArrayIterator([1, 2, 3, 4])), false)), "\n";
