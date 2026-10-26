
// SPL (Standard PHP Library) classes, written in PHP. Internal state lives in
// `__phpx_*` properties, which the engine hides like PHP's C-level fields;
// code in this file doesn't move the reported source position, so warnings
// and exceptions point at the caller, as with PHP's internal classes.

interface SeekableIterator extends Iterator {
    public function seek(int $offset): void;
}
interface OuterIterator extends Iterator {
    public function getInnerIterator(): ?Iterator;
}
interface RecursiveIterator extends Iterator {
    public function hasChildren(): bool;
    public function getChildren(): ?RecursiveIterator;
}
interface Serializable {
    public function serialize();
    public function unserialize(string $data);
}
interface SplObserver {
    public function update(SplSubject $subject): void;
}
interface SplSubject {
    public function attach(SplObserver $observer): void;
    public function detach(SplObserver $observer): void;
    public function notify(): void;
}

// ---- ArrayIterator / ArrayObject --------------------------------------------

class ArrayIterator implements SeekableIterator, ArrayAccess, Serializable, Countable
{
    public const STD_PROP_LIST = 1;
    public const ARRAY_AS_PROPS = 2;

    private $storage;
    private $__phpx_flags = 0;
    private $__phpx_keys = null;
    private $__phpx_pos = 0;

    public function __construct(array|object $array = [], int $flags = 0)
    {
        $this->storage = is_object($array) ? get_object_vars($array) : $array;
        $this->__phpx_flags = $flags;
    }

    private function __phpx_keys(): array
    {
        return $this->__phpx_keys ??= array_keys($this->storage);
    }

    public function current(): mixed
    {
        $keys = $this->__phpx_keys();
        if (!isset($keys[$this->__phpx_pos])) {
            return null;
        }
        return $this->storage[$keys[$this->__phpx_pos]] ?? null;
    }
    public function key(): string|int|null { return $this->__phpx_keys()[$this->__phpx_pos] ?? null; }
    public function next(): void { $this->__phpx_pos++; }
    public function rewind(): void { $this->__phpx_keys = null; $this->__phpx_pos = 0; }
    public function valid(): bool { return $this->__phpx_pos < count($this->__phpx_keys()); }
    public function seek(int $offset): void
    {
        if ($offset < 0 || $offset >= count($this->storage)) {
            throw new OutOfBoundsException("Seek position $offset is out of range");
        }
        $this->__phpx_keys = null;
        $this->__phpx_pos = $offset;
    }
    public function count(): int { return count($this->storage); }
    public function offsetExists(mixed $key): bool { return isset($this->storage[$key]); }
    public function offsetGet(mixed $key): mixed { return $this->storage[$key]; }
    public function offsetSet(mixed $key, mixed $value): void
    {
        if ($key === null) {
            $this->storage[] = $value;
        } else {
            $this->storage[$key] = $value;
        }
        $this->__phpx_keys = null;
    }
    public function offsetUnset(mixed $key): void { unset($this->storage[$key]); $this->__phpx_keys = null; }
    public function append(mixed $value): void { $this->storage[] = $value; $this->__phpx_keys = null; }
    public function getArrayCopy(): array { return $this->storage; }
    public function getFlags(): int { return $this->__phpx_flags; }
    public function setFlags(int $flags): void { $this->__phpx_flags = $flags; }
    public function asort(int $flags = SORT_REGULAR): bool { $this->__phpx_keys = null; return asort($this->storage, $flags); }
    public function ksort(int $flags = SORT_REGULAR): bool { $this->__phpx_keys = null; return ksort($this->storage, $flags); }
    public function uasort(callable $callback): bool { $this->__phpx_keys = null; return uasort($this->storage, $callback); }
    public function uksort(callable $callback): bool { $this->__phpx_keys = null; return uksort($this->storage, $callback); }
    public function natsort(): bool { $this->__phpx_keys = null; return natsort($this->storage); }
    public function natcasesort(): bool { $this->__phpx_keys = null; return natcasesort($this->storage); }
    public function serialize() { return serialize($this->__serialize()); }
    public function unserialize(string $data) { $this->__unserialize(unserialize($data)); }
    public function __serialize(): array { return [$this->__phpx_flags, $this->storage, [], null]; }
    public function __unserialize(array $data): void
    {
        $this->__phpx_flags = $data[0];
        $this->storage = $data[1];
        foreach ($data[2] ?? [] as $k => $v) {
            $this->$k = $v;
        }
    }
    public function __phpx_cast(): array { return $this->storage; }
}

class RecursiveArrayIterator extends ArrayIterator implements RecursiveIterator
{
    public const CHILD_ARRAYS_ONLY = 4;

    public function hasChildren(): bool
    {
        $c = $this->current();
        return is_array($c) || (is_object($c) && !($this->getFlags() & self::CHILD_ARRAYS_ONLY));
    }
    public function getChildren(): ?RecursiveArrayIterator
    {
        return new static($this->current(), $this->getFlags());
    }
}

class ArrayObject implements IteratorAggregate, ArrayAccess, Serializable, Countable
{
    public const STD_PROP_LIST = 1;
    public const ARRAY_AS_PROPS = 2;

    private $storage;
    private $__phpx_flags = 0;
    private $__phpx_iterator = ArrayIterator::class;

    public function __construct(array|object $array = [], int $flags = 0, string $iteratorClass = ArrayIterator::class)
    {
        $this->storage = is_object($array) ? ($array instanceof ArrayObject || $array instanceof ArrayIterator ? $array->getArrayCopy() : get_object_vars($array)) : $array;
        $this->__phpx_flags = $flags;
        $this->__phpx_iterator = $iteratorClass;
    }

    public function getIterator(): Iterator { return new ($this->__phpx_iterator)($this->storage); }
    public function count(): int { return count($this->storage); }
    public function offsetExists(mixed $key): bool { return isset($this->storage[$key]); }
    public function offsetGet(mixed $key): mixed { return $this->storage[$key]; }
    public function offsetSet(mixed $key, mixed $value): void
    {
        if ($key === null) {
            $this->storage[] = $value;
        } else {
            $this->storage[$key] = $value;
        }
    }
    public function offsetUnset(mixed $key): void { unset($this->storage[$key]); }
    public function append(mixed $value): void { $this->storage[] = $value; }
    public function getArrayCopy(): array { return $this->storage; }
    public function exchangeArray(array|object $array): array
    {
        $old = $this->storage;
        $this->storage = is_object($array) ? get_object_vars($array) : $array;
        return $old;
    }
    public function getFlags(): int { return $this->__phpx_flags; }
    public function setFlags(int $flags): void { $this->__phpx_flags = $flags; }
    public function getIteratorClass(): string { return $this->__phpx_iterator; }
    public function setIteratorClass(string $iteratorClass): void { $this->__phpx_iterator = $iteratorClass; }
    public function asort(int $flags = SORT_REGULAR): bool { return asort($this->storage, $flags); }
    public function ksort(int $flags = SORT_REGULAR): bool { return ksort($this->storage, $flags); }
    public function uasort(callable $callback): bool { return uasort($this->storage, $callback); }
    public function uksort(callable $callback): bool { return uksort($this->storage, $callback); }
    public function natsort(): bool { return natsort($this->storage); }
    public function natcasesort(): bool { return natcasesort($this->storage); }
    public function serialize() { return serialize($this->__serialize()); }
    public function unserialize(string $data) { $this->__unserialize(unserialize($data)); }
    public function __serialize(): array
    {
        return [$this->__phpx_flags, $this->storage, [], $this->__phpx_iterator === ArrayIterator::class ? null : $this->__phpx_iterator];
    }
    public function __unserialize(array $data): void
    {
        $this->__phpx_flags = $data[0];
        $this->storage = $data[1];
        foreach ($data[2] ?? [] as $k => $v) {
            $this->$k = $v;
        }
        $this->__phpx_iterator = $data[3] ?? ArrayIterator::class;
    }
    public function __phpx_cast(): array { return $this->storage; }
}

// ---- Iterators ---------------------------------------------------------------

class EmptyIterator implements Iterator
{
    public function current(): never { throw new BadMethodCallException("Accessing the value of an EmptyIterator"); }
    public function key(): never { throw new BadMethodCallException("Accessing the key of an EmptyIterator"); }
    public function next(): void {}
    public function rewind(): void {}
    public function valid(): bool { return false; }
}

class IteratorIterator implements OuterIterator
{
    // The inner iterator, the cached element and the stepping methods
    // are native (spl_iter.rs).

    public function __construct(Traversable $iterator, ?string $class = null)
    {
        while ($iterator instanceof IteratorAggregate) {
            $iterator = $iterator->getIterator();
        }
        $this->__phpx_init($iterator);
    }

    public function __call($name, $args) { return $this->getInnerIterator()->$name(...$args); }
}

abstract class FilterIterator extends IteratorIterator
{
    // rewind() and next() skip what accept() rejects (spl_iter.rs).

    abstract public function accept(): bool;

    public function __construct(Iterator $iterator) { parent::__construct($iterator); }
}

class CallbackFilterIterator extends FilterIterator
{
    private $__phpx_callback;

    public function __construct(Iterator $iterator, callable $callback)
    {
        parent::__construct($iterator);
        $this->__phpx_callback = $callback;
    }
    public function accept(): bool
    {
        return (bool)($this->__phpx_callback)($this->current(), $this->key(), $this->getInnerIterator());
    }
}

abstract class RecursiveFilterIterator extends FilterIterator implements RecursiveIterator
{
    public function __construct(RecursiveIterator $iterator) { parent::__construct($iterator); }
    public function hasChildren(): bool { return $this->getInnerIterator()->hasChildren(); }
    public function getChildren(): ?RecursiveFilterIterator { return new static($this->getInnerIterator()->getChildren()); }
}

class RecursiveCallbackFilterIterator extends CallbackFilterIterator implements RecursiveIterator
{
    private $__phpx_cb;

    public function __construct(RecursiveIterator $iterator, callable $callback)
    {
        parent::__construct($iterator, $callback);
        $this->__phpx_cb = $callback;
    }
    public function hasChildren(): bool { return $this->getInnerIterator()->hasChildren(); }
    public function getChildren(): RecursiveCallbackFilterIterator
    {
        return new static($this->getInnerIterator()->getChildren(), $this->__phpx_cb);
    }
}

class ParentIterator extends RecursiveFilterIterator
{
    public function accept(): bool { return $this->getInnerIterator()->hasChildren(); }
}

class LimitIterator extends IteratorIterator
{
    private $__phpx_offset;
    private $__phpx_limit;
    private $__phpx_at = 0;

    public function __construct(Iterator $iterator, int $offset = 0, int $limit = -1)
    {
        if ($offset < 0) {
            throw new ValueError("LimitIterator::__construct(): Argument #2 (\$offset) must be greater than or equal to 0");
        }
        if ($limit < -1) {
            throw new ValueError("LimitIterator::__construct(): Argument #3 (\$limit) must be greater than or equal to -1");
        }
        parent::__construct($iterator);
        $this->__phpx_offset = $offset;
        $this->__phpx_limit = $limit;
    }
    public function rewind(): void
    {
        parent::rewind();
        $this->__phpx_at = 0;
        while ($this->__phpx_at < $this->__phpx_offset && parent::valid()) {
            parent::next();
            $this->__phpx_at++;
        }
    }
    public function valid(): bool
    {
        return ($this->__phpx_limit == -1 || $this->__phpx_at < $this->__phpx_offset + $this->__phpx_limit) && parent::valid();
    }
    public function next(): void { parent::next(); $this->__phpx_at++; }
    public function getPosition(): int { return $this->__phpx_at; }
    public function seek(int $offset): int
    {
        $this->rewind();
        while ($this->__phpx_at < $offset && $this->valid()) {
            $this->next();
        }
        return $this->__phpx_at;
    }
}

class InfiniteIterator extends IteratorIterator
{
    public function next(): void
    {
        parent::next();
        if (!parent::valid()) {
            parent::rewind();
        }
    }
}

class NoRewindIterator extends IteratorIterator
{
    private $__phpx_started = false;

    public function rewind(): void
    {
        if (!$this->__phpx_started) {
            $this->__phpx_started = true;
            $this->__phpx_fetch();
        }
    }
}

class AppendIterator extends IteratorIterator
{
    private $__phpx_its = [];
    private $__phpx_idx = 0;

    public function __construct() {}
    public function append(Iterator $iterator): void
    {
        $this->__phpx_its[] = $iterator;
        if (count($this->__phpx_its) == 1) {
            $iterator->rewind();
            $this->__phpx_idx = 0;
            $this->__phpx_settle();
        }
    }
    public function getInnerIterator(): ?Iterator { return $this->__phpx_its[$this->__phpx_idx] ?? null; }
    public function getIteratorIndex(): ?int { return isset($this->__phpx_its[$this->__phpx_idx]) ? $this->__phpx_idx : null; }
    public function getArrayIterator(): ArrayIterator { return new ArrayIterator($this->__phpx_its); }
    private function __phpx_settle(): void
    {
        while (isset($this->__phpx_its[$this->__phpx_idx]) && !$this->__phpx_its[$this->__phpx_idx]->valid()) {
            $this->__phpx_idx++;
            if (isset($this->__phpx_its[$this->__phpx_idx])) {
                $this->__phpx_its[$this->__phpx_idx]->rewind();
            }
        }
    }
    public function rewind(): void
    {
        $this->__phpx_idx = 0;
        if (isset($this->__phpx_its[0])) {
            $this->__phpx_its[0]->rewind();
        }
        $this->__phpx_settle();
    }
    public function valid(): bool { return isset($this->__phpx_its[$this->__phpx_idx]) && $this->__phpx_its[$this->__phpx_idx]->valid(); }
    public function key(): mixed { return $this->valid() ? $this->__phpx_its[$this->__phpx_idx]->key() : null; }
    public function current(): mixed { return $this->valid() ? $this->__phpx_its[$this->__phpx_idx]->current() : null; }
    public function next(): void
    {
        if (isset($this->__phpx_its[$this->__phpx_idx])) {
            $this->__phpx_its[$this->__phpx_idx]->next();
            $this->__phpx_settle();
        }
    }
}

class CachingIterator extends IteratorIterator implements ArrayAccess, Countable, Stringable
{
    public const CALL_TOSTRING = 1;
    public const CATCH_GET_CHILD = 16;
    public const TOSTRING_USE_KEY = 2;
    public const TOSTRING_USE_CURRENT = 4;
    public const TOSTRING_USE_INNER = 8;
    public const FULL_CACHE = 256;

    private $__phpx_flags;
    private $__phpx_has = false;
    private $__phpx_k = null;
    private $__phpx_c = null;
    private $__phpx_cache = [];

    public function __construct(Iterator $iterator, int $flags = CachingIterator::CALL_TOSTRING)
    {
        parent::__construct($iterator);
        $this->__phpx_flags = $flags;
    }
    private function __phpx_step(): void
    {
        $this->__phpx_has = parent::valid();
        if ($this->__phpx_has) {
            $this->__phpx_k = parent::key();
            $this->__phpx_c = parent::current();
            if ($this->__phpx_flags & self::FULL_CACHE) {
                $this->__phpx_cache[$this->__phpx_k] = $this->__phpx_c;
            }
            parent::next();
        } else {
            $this->__phpx_k = null;
            $this->__phpx_c = null;
        }
    }
    public function rewind(): void { parent::rewind(); $this->__phpx_cache = []; $this->__phpx_step(); }
    public function valid(): bool { return $this->__phpx_has; }
    public function key(): mixed { return $this->__phpx_k; }
    public function current(): mixed { return $this->__phpx_c; }
    public function next(): void { $this->__phpx_step(); }
    public function hasNext(): bool { return parent::valid(); }
    public function getFlags(): int { return $this->__phpx_flags; }
    public function setFlags(int $flags): void { $this->__phpx_flags = $flags; }
    public function getCache(): array { return $this->__phpx_cache; }
    public function count(): int { return count($this->__phpx_cache); }
    public function offsetGet($key): mixed { return $this->__phpx_cache[$key] ?? null; }
    public function offsetSet($key, mixed $value): void { $this->__phpx_cache[$key] = $value; }
    public function offsetExists($key): bool { return isset($this->__phpx_cache[$key]); }
    public function offsetUnset($key): void { unset($this->__phpx_cache[$key]); }
    public function __toString(): string
    {
        if ($this->__phpx_flags & self::TOSTRING_USE_KEY) {
            return (string)$this->__phpx_k;
        }
        return (string)$this->__phpx_c;
    }
}

class RecursiveCachingIterator extends CachingIterator implements RecursiveIterator
{
    public function hasChildren(): bool { return $this->getInnerIterator()->hasChildren(); }
    public function getChildren(): ?RecursiveCachingIterator { return new static($this->getInnerIterator()->getChildren(), $this->getFlags()); }
}

class RecursiveIteratorIterator implements OuterIterator
{
    public const LEAVES_ONLY = 0;
    public const SELF_FIRST = 1;
    public const CHILD_FIRST = 2;
    public const CATCH_GET_CHILD = 16;

    // The walk's state and the stepping methods are native (spl_iter.rs);
    // the hooks below run only when a subclass overrides them.

    public function __construct(Traversable $iterator, int $mode = self::LEAVES_ONLY, int $flags = 0)
    {
        while ($iterator instanceof IteratorAggregate) {
            $iterator = $iterator->getIterator();
        }
        $this->__phpx_init($iterator, $mode);
    }

    public function beginIteration() {}
    public function endIteration() {}
    public function beginChildren(): void {}
    public function endChildren(): void {}
    public function nextElement(): void {}
    public function __call($name, $args) { return $this->getInnerIterator()->$name(...$args); }
}

function iterator_count(Traversable|array $iterator): int
{
    if (is_array($iterator)) {
        return count($iterator);
    }
    $n = 0;
    foreach ($iterator as $_) {
        $n++;
    }
    return $n;
}

function iterator_apply(Traversable $iterator, callable $callback, ?array $args = null): int
{
    $n = 0;
    foreach ($iterator as $_) {
        $n++;
        if (!$callback(...($args ?? []))) {
            break;
        }
    }
    return $n;
}

// ---- Doubly linked list, stack, queue ----------------------------------------

class SplDoublyLinkedList implements Iterator, Countable, ArrayAccess, Serializable
{
    public const IT_MODE_LIFO = 2;
    public const IT_MODE_FIFO = 0;
    public const IT_MODE_DELETE = 1;
    public const IT_MODE_KEEP = 0;

    private $flags = 0;
    private $dllist = [];
    private $__phpx_pos = 0;

    public function add(int $index, mixed $value): void
    {
        $n = count($this->dllist);
        if ($index < 0 || $index > $n) {
            throw new OutOfRangeException("SplDoublyLinkedList::add(): Argument #1 (\$index) is out of range");
        }
        if ($index == $n) {
            $this->dllist[] = $value;
            return;
        }
        // Inserted before the element at $index (counted from the end in
        // LIFO mode), as PHP's list does.
        $at = ($this->flags & self::IT_MODE_LIFO) ? $n - 1 - $index : $index;
        array_splice($this->dllist, $at, 0, [$value]);
    }
    public function pop(): mixed
    {
        if (!$this->dllist) {
            throw new RuntimeException("Can't pop from an empty datastructure");
        }
        return array_pop($this->dllist);
    }
    public function shift(): mixed
    {
        if (!$this->dllist) {
            throw new RuntimeException("Can't shift from an empty datastructure");
        }
        return array_shift($this->dllist);
    }
    public function push(mixed $value): void { $this->dllist[] = $value; }
    public function unshift(mixed $value): void { array_unshift($this->dllist, $value); }
    public function top(): mixed
    {
        if (!$this->dllist) {
            throw new RuntimeException("Can't peek at an empty datastructure");
        }
        return $this->dllist[count($this->dllist) - 1];
    }
    public function bottom(): mixed
    {
        if (!$this->dllist) {
            throw new RuntimeException("Can't peek at an empty datastructure");
        }
        return $this->dllist[0];
    }
    public function isEmpty(): bool { return !$this->dllist; }
    public function count(): int { return count($this->dllist); }
    public function setIteratorMode(int $mode): int
    {
        if (($this->flags & 4) && ($mode & self::IT_MODE_LIFO) != ($this->flags & self::IT_MODE_LIFO)) {
            throw new RuntimeException("Iterators' LIFO/FIFO modes for SplStack/SplQueue objects are frozen");
        }
        $this->flags = ($mode & 3) | ($this->flags & 4);
        return $this->flags;
    }
    public function getIteratorMode(): int { return $this->flags; }
    /** SplStack / SplQueue: their LIFO/FIFO mode can't change. */
    protected function __phpx_freeze(): void { $this->flags |= 4; }
    private function __phpx_index($index, string $method): int
    {
        $n = count($this->dllist);
        if (!is_numeric($index) || $index < 0 || $index >= $n) {
            throw new OutOfRangeException("SplDoublyLinkedList::$method(): Argument #1 (\$index) is out of range");
        }
        // LIFO mode counts offsets from the end.
        return ($this->flags & self::IT_MODE_LIFO) ? $n - 1 - (int)$index : (int)$index;
    }
    public function offsetExists($index): bool { return is_numeric($index) && $index >= 0 && $index < count($this->dllist); }
    public function offsetGet($index): mixed { return $this->dllist[$this->__phpx_index($index, 'offsetGet')]; }
    public function offsetSet($index, mixed $value): void
    {
        if ($index === null) {
            $this->dllist[] = $value;
        } else {
            $this->dllist[$this->__phpx_index($index, 'offsetSet')] = $value;
        }
    }
    public function offsetUnset($index): void
    {
        array_splice($this->dllist, $this->__phpx_index($index, 'offsetUnset'), 1);
    }
    public function rewind(): void
    {
        $this->__phpx_pos = ($this->flags & self::IT_MODE_LIFO) ? count($this->dllist) - 1 : 0;
    }
    public function valid(): bool { return $this->__phpx_pos >= 0 && $this->__phpx_pos < count($this->dllist); }
    public function current(): mixed { return $this->dllist[$this->__phpx_pos] ?? null; }
    public function key(): int { return $this->__phpx_pos; }
    public function next(): void
    {
        if ($this->flags & self::IT_MODE_DELETE) {
            if ($this->flags & self::IT_MODE_LIFO) {
                array_pop($this->dllist);
                $this->__phpx_pos = count($this->dllist) - 1;
            } else {
                array_shift($this->dllist);
            }
            return;
        }
        $this->__phpx_pos += ($this->flags & self::IT_MODE_LIFO) ? -1 : 1;
    }
    public function prev(): void { $this->__phpx_pos += ($this->flags & self::IT_MODE_LIFO) ? 1 : -1; }
    public function serialize() { return serialize($this->__serialize()); }
    public function unserialize(string $data) { $this->__unserialize(unserialize($data)); }
    public function __serialize(): array { return [$this->flags, $this->dllist, []]; }
    public function __unserialize(array $data): void { [$this->flags, $this->dllist] = $data; }
}

class SplQueue extends SplDoublyLinkedList
{
    public function __construct() { $this->setIteratorMode(self::IT_MODE_FIFO); $this->__phpx_freeze(); }
    public function enqueue(mixed $value): void { $this->push($value); }
    public function dequeue(): mixed { return $this->shift(); }
}

class SplStack extends SplDoublyLinkedList
{
    public function __construct() { $this->setIteratorMode(self::IT_MODE_LIFO); $this->__phpx_freeze(); }
}

// ---- Heaps -------------------------------------------------------------------

abstract class SplHeap implements Iterator, Countable
{
    private $flags = 0;
    private $isCorrupted = false;
    private $heap = [];

    abstract protected function compare(mixed $value1, mixed $value2): int;

    /** PHP's binary heap (spl_heap.c), so equal elements come out in the same order. */
    protected function __phpx_cmp($a, $b): int { return $this->compare($a, $b); }
    public function insert(mixed $value): true
    {
        $h = &$this->heap;
        for ($i = count($h); $i > 0 && $this->__phpx_cmp($h[($i - 1) >> 1], $value) < 0; $i = ($i - 1) >> 1) {
            $h[$i] = $h[($i - 1) >> 1];
        }
        $h[$i] = $value;
        return true;
    }
    public function extract(): mixed
    {
        if (!$this->heap) {
            throw new RuntimeException("Can't extract from an empty heap");
        }
        $h = &$this->heap;
        $count = count($h);
        $top = $h[0];
        $limit = ($count - 1) >> 1;
        $bottom = $h[$count - 1];
        for ($i = 0; $i < $limit; $i = $j) {
            $j = $i * 2 + 1;
            if ($j != $count && $this->__phpx_cmp($h[$j + 1], $h[$j]) > 0) {
                $j++;
            }
            if ($this->__phpx_cmp($bottom, $h[$j]) < 0) {
                $h[$i] = $h[$j];
            } else {
                break;
            }
        }
        $h[$i] = $bottom;
        unset($h[$count - 1]);
        return $top;
    }
    public function top(): mixed
    {
        if (!$this->heap) {
            throw new RuntimeException("Can't peek at an empty heap");
        }
        return $this->heap[0];
    }
    public function count(): int { return count($this->heap); }
    public function isEmpty(): bool { return !$this->heap; }
    public function rewind(): void {}
    public function valid(): bool { return (bool)$this->heap; }
    public function current(): mixed { return $this->heap[0] ?? null; }
    public function key(): int { return count($this->heap) - 1; }
    public function next(): void { if ($this->heap) { $this->extract(); } }
    public function recoverFromCorruption(): true { $this->isCorrupted = false; return true; }
    public function isCorrupted(): bool { return $this->isCorrupted; }
    public function __debugInfo(): array
    {
        return ["\0SplHeap\0flags" => $this->flags, "\0SplHeap\0isCorrupted" => $this->isCorrupted, "\0SplHeap\0heap" => $this->heap];
    }
}

class SplMinHeap extends SplHeap
{
    protected function compare(mixed $value1, mixed $value2): int { return $value2 <=> $value1; }
}

class SplMaxHeap extends SplHeap
{
    protected function compare(mixed $value1, mixed $value2): int { return $value1 <=> $value2; }
}

class SplPriorityQueue implements Iterator, Countable
{
    public const EXTR_BOTH = 3;
    public const EXTR_PRIORITY = 2;
    public const EXTR_DATA = 1;

    private $flags = 1;
    private $isCorrupted = false;
    private $heap = [];

    public function compare(mixed $priority1, mixed $priority2): int { return $priority1 <=> $priority2; }
    public function insert(mixed $value, mixed $priority)
    {
        $e = ['data' => $value, 'priority' => $priority];
        $h = &$this->heap;
        for ($i = count($h); $i > 0 && $this->compare($h[($i - 1) >> 1]['priority'], $priority) < 0; $i = ($i - 1) >> 1) {
            $h[$i] = $h[($i - 1) >> 1];
        }
        $h[$i] = $e;
        return true;
    }
    private function __phpx_out(array $e): mixed
    {
        return match ($this->flags & 3) {
            self::EXTR_BOTH => $e,
            self::EXTR_PRIORITY => $e['priority'],
            default => $e['data'],
        };
    }
    public function extract(): mixed
    {
        if (!$this->heap) {
            throw new RuntimeException("Can't extract from an empty heap");
        }
        $h = &$this->heap;
        $count = count($h);
        $top = $h[0];
        $limit = ($count - 1) >> 1;
        $bottom = $h[$count - 1];
        for ($i = 0; $i < $limit; $i = $j) {
            $j = $i * 2 + 1;
            if ($j != $count && $this->compare($h[$j + 1]['priority'], $h[$j]['priority']) > 0) {
                $j++;
            }
            if ($this->compare($bottom['priority'], $h[$j]['priority']) < 0) {
                $h[$i] = $h[$j];
            } else {
                break;
            }
        }
        $h[$i] = $bottom;
        unset($h[$count - 1]);
        return $this->__phpx_out($top);
    }
    public function top(): mixed
    {
        if (!$this->heap) {
            throw new RuntimeException("Can't peek at an empty heap");
        }
        return $this->__phpx_out($this->heap[0]);
    }
    public function setExtractFlags(int $flags): int
    {
        if (($flags & 3) == 0) {
            throw new RuntimeException("Must specify at least one extract flag");
        }
        return $this->flags = $flags & 3;
    }
    public function getExtractFlags(): int { return $this->flags; }
    public function count(): int { return count($this->heap); }
    public function isEmpty(): bool { return !$this->heap; }
    public function rewind(): void {}
    public function valid(): bool { return (bool)$this->heap; }
    public function current(): mixed { return $this->heap ? $this->__phpx_out($this->heap[0]) : null; }
    public function key(): int { return count($this->heap) - 1; }
    public function next(): void { if ($this->heap) { $this->extract(); } }
    public function recoverFromCorruption(): true { $this->isCorrupted = false; return true; }
    public function isCorrupted(): bool { return $this->isCorrupted; }
}

// ---- Fixed array ---------------------------------------------------------------

class SplFixedArray implements IteratorAggregate, ArrayAccess, Countable, JsonSerializable
{
    private $__phpx_data = [];

    public function __construct(int $size = 0)
    {
        if ($size < 0) {
            throw new ValueError("SplFixedArray::__construct(): Argument #1 (\$size) must be greater than or equal to 0");
        }
        $this->__phpx_data = $size ? array_fill(0, $size, null) : [];
    }
    private function __phpx_at($index): int
    {
        if (is_string($index) && is_numeric($index)) {
            $index = (int)$index;
        }
        if (!is_int($index) && !is_float($index) && !is_bool($index)) {
            throw new TypeError("Cannot access offset of type " . get_debug_type($index) . " on SplFixedArray");
        }
        $index = (int)$index;
        if ($index < 0 || $index >= count($this->__phpx_data)) {
            throw new OutOfBoundsException("Index invalid or out of range");
        }
        return $index;
    }
    public function offsetExists($index): bool
    {
        return is_numeric($index) && $index >= 0 && $index < count($this->__phpx_data) && $this->__phpx_data[(int)$index] !== null;
    }
    public function offsetGet($index): mixed { return $this->__phpx_data[$this->__phpx_at($index)]; }
    public function offsetSet($index, mixed $value): void
    {
        if ($index === null) {
            throw new OutOfBoundsException("Index invalid or out of range");
        }
        $this->__phpx_data[$this->__phpx_at($index)] = $value;
    }
    public function offsetUnset($index): void { $this->__phpx_data[$this->__phpx_at($index)] = null; }
    public function count(): int { return count($this->__phpx_data); }
    public function getSize(): int { return count($this->__phpx_data); }
    public function setSize(int $size): bool
    {
        if ($size < 0) {
            throw new ValueError("SplFixedArray::setSize(): Argument #1 (\$size) must be greater than or equal to 0");
        }
        $n = count($this->__phpx_data);
        if ($size < $n) {
            $this->__phpx_data = array_slice($this->__phpx_data, 0, $size);
        } else {
            for ($i = $n; $i < $size; $i++) {
                $this->__phpx_data[$i] = null;
            }
        }
        return true;
    }
    public function toArray(): array { return $this->__phpx_data; }
    public static function fromArray(array $array, bool $preserveKeys = true): SplFixedArray
    {
        $fa = new SplFixedArray();
        if (!$array) {
            return $fa;
        }
        if ($preserveKeys) {
            foreach ($array as $k => $_) {
                if (!is_int($k) || $k < 0) {
                    throw new ValueError("array must contain only positive integer keys");
                }
            }
            $fa->setSize(max(array_keys($array)) + 1);
            foreach ($array as $k => $v) {
                $fa[$k] = $v;
            }
        } else {
            $fa->setSize(count($array));
            $i = 0;
            foreach ($array as $v) {
                $fa[$i++] = $v;
            }
        }
        return $fa;
    }
    public function getIterator(): Iterator { return new ArrayIterator($this->__phpx_data); }
    public function jsonSerialize(): array { return $this->__phpx_data; }
    public function __debugInfo(): array { return $this->__phpx_data; }
    public function __serialize(): array { return $this->__phpx_data; }
    public function __unserialize(array $data): void { $this->__phpx_data = array_values($data); }
    public function __wakeup(): void {}
}

// ---- Object storage -----------------------------------------------------------

class SplObjectStorage implements Countable, SeekableIterator, Serializable, ArrayAccess
{
    /** [handle => ['obj' => ..., 'inf' => ...]] in insertion order. */
    private $__phpx_map = [];
    private $__phpx_pos = 0;
    private $__phpx_keys = null;

    public function attach(object $object, mixed $info = null): void
    {
        __phpx_deprecated("Method SplObjectStorage::attach() is deprecated since 8.5, use method SplObjectStorage::offsetSet() instead");
        $this->offsetSet($object, $info);
    }
    public function detach(object $object): void
    {
        __phpx_deprecated("Method SplObjectStorage::detach() is deprecated since 8.5, use method SplObjectStorage::offsetUnset() instead");
        $this->offsetUnset($object);
    }
    public function contains(object $object): bool
    {
        __phpx_deprecated("Method SplObjectStorage::contains() is deprecated since 8.5, use method SplObjectStorage::offsetExists() instead");
        return $this->offsetExists($object);
    }
    public function addAll(SplObjectStorage $storage): int
    {
        foreach ($storage->__phpx_map as $h => $e) {
            $this->__phpx_map[$h] = $e;
        }
        $this->__phpx_keys = null;
        return count($this->__phpx_map);
    }
    public function removeAll(SplObjectStorage $storage): int
    {
        foreach ($storage->__phpx_map as $h => $_) {
            unset($this->__phpx_map[$h]);
        }
        $this->__phpx_keys = null;
        return count($this->__phpx_map);
    }
    public function removeAllExcept(SplObjectStorage $storage): int
    {
        foreach ($this->__phpx_map as $h => $_) {
            if (!isset($storage->__phpx_map[$h])) {
                unset($this->__phpx_map[$h]);
            }
        }
        $this->__phpx_keys = null;
        return count($this->__phpx_map);
    }
    public function getInfo(): mixed
    {
        $k = $this->__phpx_keys()[$this->__phpx_pos] ?? null;
        return $k === null ? null : $this->__phpx_map[$k]['inf'];
    }
    public function setInfo(mixed $info): void
    {
        $k = $this->__phpx_keys()[$this->__phpx_pos] ?? null;
        if ($k !== null) {
            $this->__phpx_map[$k]['inf'] = $info;
        }
    }
    public function getHash(object $object): string { return spl_object_hash($object); }
    public function count(int $mode = COUNT_NORMAL): int
    {
        if ($mode == COUNT_RECURSIVE) {
            $n = 0;
            foreach ($this->__phpx_map as $e) {
                $n += 1 + (is_countable($e['inf']) ? count($e['inf'], COUNT_RECURSIVE) : 0);
            }
            return $n;
        }
        return count($this->__phpx_map);
    }
    public function offsetExists($object): bool { return isset($this->__phpx_map[spl_object_id($object)]); }
    public function offsetGet($object): mixed
    {
        $h = spl_object_id($object);
        if (!isset($this->__phpx_map[$h])) {
            throw new UnexpectedValueException("Object not found");
        }
        return $this->__phpx_map[$h]['inf'];
    }
    public function offsetSet($object, mixed $info = null): void
    {
        $h = spl_object_id($object);
        if (isset($this->__phpx_map[$h])) {
            $this->__phpx_map[$h]['inf'] = $info;
        } else {
            $this->__phpx_map[$h] = ['obj' => $object, 'inf' => $info];
            $this->__phpx_keys = null;
        }
    }
    public function offsetUnset($object): void
    {
        unset($this->__phpx_map[spl_object_id($object)]);
        $this->__phpx_keys = null;
    }
    private function __phpx_keys(): array { return $this->__phpx_keys ??= array_keys($this->__phpx_map); }
    public function rewind(): void { $this->__phpx_keys = null; $this->__phpx_pos = 0; }
    public function valid(): bool { return $this->__phpx_pos < count($this->__phpx_keys()); }
    public function key(): int { return $this->__phpx_pos; }
    public function current(): object
    {
        $k = $this->__phpx_keys()[$this->__phpx_pos] ?? null;
        if ($k === null) {
            throw new RuntimeException("Called current() on invalid iterator");
        }
        return $this->__phpx_map[$k]['obj'];
    }
    public function next(): void { $this->__phpx_pos++; }
    public function seek(int $offset): void
    {
        if ($offset < 0 || $offset >= count($this->__phpx_map)) {
            throw new OutOfBoundsException("Seek position $offset is out of range");
        }
        $this->__phpx_pos = $offset;
    }
    public function serialize() { return serialize($this->__serialize()); }
    public function unserialize(string $data) { $this->__unserialize(unserialize($data)); }
    public function __serialize(): array
    {
        $flat = [];
        foreach ($this->__phpx_map as $e) {
            $flat[] = $e['obj'];
            $flat[] = $e['inf'];
        }
        return [$flat, []];
    }
    public function __unserialize(array $data): void
    {
        $this->__phpx_map = [];
        for ($i = 0; $i + 1 < count($data[0]); $i += 2) {
            $this->offsetSet($data[0][$i], $data[0][$i + 1]);
        }
    }
    public function __debugInfo(): array { return ["\0SplObjectStorage\0storage" => array_values($this->__phpx_map)]; }
}
