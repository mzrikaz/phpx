<?php
function show($l, $v) { echo "== $l\n"; var_dump($v); }
$s = new SplStack(); $s->push(1); $s->push(2); $s[] = 3;
show('stack', $s);
echo $s->top(), $s->pop(), count($s), $s->isEmpty() ? 'e' : 'n', "\n";
foreach ($s as $k => $v) echo "$k:$v "; echo "\n";
$q = new SplQueue(); $q->enqueue('a'); $q->enqueue('b'); $q->push('c');
show('queue', $q);
echo $q->dequeue(), $q->bottom(), $q->top(), count($q), "\n";
foreach ($q as $k => $v) echo "$k:$v "; echo "\n";
try { (new SplStack)->pop(); } catch (RuntimeException $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { (new SplQueue)->dequeue(); } catch (RuntimeException $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { (new SplStack)->top(); } catch (RuntimeException $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$l = new SplDoublyLinkedList(); foreach ([5, 6, 7] as $x) $l->push($x);
$l->unshift(4); $l->offsetSet(1, 50); echo $l->offsetGet(1), $l[0], count($l), implode(",", iterator_to_array($l, false)), "\n";
$l->setIteratorMode(SplDoublyLinkedList::IT_MODE_LIFO); foreach ($l as $v) echo $v, " "; echo "\n";
$l->add(2, 99); echo implode(",", iterator_to_array($l, false)), "\n";
try { $l->offsetGet(10); } catch (OutOfRangeException $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$h = new SplMinHeap(); foreach ([5, 1, 8, 3, 9, 2] as $x) $h->insert($x);
echo $h->top(), count($h), " "; foreach ($h as $v) echo $v, " "; echo count($h), "\n";
$h = new SplMaxHeap(); foreach ([5, 1, 8, 3] as $x) $h->insert($x);
show('maxheap', $h);
echo $h->extract(), $h->extract(), "\n";
try { (new SplMinHeap)->extract(); } catch (RuntimeException $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$pq = new SplPriorityQueue(); foreach ([['a', 3], ['b', 1], ['c', 3], ['d', 2], ['e', 3]] as [$v, $p]) $pq->insert($v, $p);
echo count($pq), " ", $pq->top(), " "; while (!$pq->isEmpty()) echo $pq->extract(); echo "\n";
$pq = new SplPriorityQueue(); $pq->setExtractFlags(SplPriorityQueue::EXTR_BOTH); $pq->insert('x', 10); $pq->insert('y', 20);
var_dump($pq->extract());
class TaskHeap extends SplHeap { protected function compare($a, $b): int { return strlen($b) <=> strlen($a); } }
$th = new TaskHeap; foreach (['ccc', 'a', 'bb'] as $x) $th->insert($x); foreach ($th as $x) echo $x, " "; echo "\n";
$fa = new SplFixedArray(3); $fa[0] = 'x'; $fa[2] = 'z';
show('fixed', $fa);
echo $fa->getSize(), count($fa), json_encode($fa->toArray()), "\n";
foreach ($fa as $k => $v) echo "$k=", var_export($v, true), " "; echo "\n";
try { $fa[5] = 1; } catch (RuntimeException $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$fa->setSize(4); echo $fa->getSize(), "\n";
$fb = SplFixedArray::fromArray([1, 2, 3]); echo array_sum($fb->toArray()), "\n";
$st = new SplObjectStorage(); $o1 = new stdClass; $o2 = new stdClass; $o1->id = 1; $o2->id = 2;
@$st->attach($o1); @$st->attach($o2, "data2"); $st[$o1] = "data1";
echo count($st), @$st->contains($o1) ? 'y' : 'n', $st[$o2], "\n";
foreach ($st as $i => $obj) { echo $i, ":", $obj->id, "=", $st->getInfo(), " "; } echo "\n";
@$st->detach($o1); echo count($st), @$st->contains($o1) ? 'y' : 'n', "\n";
show('storage', $st);
echo iterator_count(new ArrayIterator([1, 2, 3])), " ", count(iterator_to_array(new EmptyIterator())), "\n";
function gen() { yield 1; yield 2; }
echo implode(",", iterator_to_array(new NoRewindIterator(gen()))), "\n";
class Evens extends FilterIterator { public function accept(): bool { return $this->current() % 2 == 0; } }
echo implode(",", iterator_to_array(new Evens(new ArrayIterator([1, 2, 3, 4])), false)), "\n";
echo iterator_apply($it = new ArrayIterator([1, 2, 3]), function () use (&$it) { echo $it->current(); return true; }, []), "\n";
