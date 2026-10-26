<?php
// Built-in classes written in phpx itself. Methods with no body here are
// provided natively (crates/interp/src/native.rs).

interface Traversable {}
interface Iterator extends Traversable {
    public function current(): mixed;
    public function key(): mixed;
    public function next(): void;
    public function rewind(): void;
    public function valid(): bool;
}
interface IteratorAggregate extends Traversable {
    public function getIterator(): Iterator;
}
interface ArrayAccess {
    public function offsetExists(mixed $offset): bool;
    public function offsetGet(mixed $offset): mixed;
    public function offsetSet(mixed $offset, mixed $value): void;
    public function offsetUnset(mixed $offset): void;
}
interface Countable {
    public function count(): int;
}
interface Stringable {}
interface JsonSerializable {
    public function jsonSerialize(): mixed;
}
interface UnitEnum {}
interface BackedEnum extends UnitEnum {}
interface Throwable extends Stringable {}

class stdClass {}

final class Closure {}

final class Generator implements Iterator {}

/** @template T */
final class Promise {}

final class Concurrency {}

final class Fiber {}

final class FiberError extends Error {}

final class WeakReference
{
    private function __construct() {}
}

final class WeakMap implements ArrayAccess, Countable, IteratorAggregate
{
    public function getIterator(): Iterator
    {
        foreach (__phpx_weakmap_entries($this) as [$k, $v]) {
            yield $k => $v;
        }
    }
}

class Exception implements Throwable
{
    protected $message = '';
    private string $string = '';
    protected $code = 0;
    protected string $file = '';
    protected int $line = 0;
    private array $trace = [];
    private ?Throwable $previous = null;

    public function __construct($message = '', $code = 0, ?Throwable $previous = null)
    {
        $this->message = $message;
        $this->code = $code;
        $this->previous = $previous;
    }

    final public function getMessage(): string { return $this->message; }
    final public function getCode() { return $this->code; }
    final public function getPrevious(): ?Throwable { return $this->previous; }
    final public function getFile(): string { return $this->file; }
    final public function getLine(): int { return $this->line; }
    final public function getTrace(): array { return $this->trace; }
    final public function getTraceAsString(): string { return __phpx_trace_string($this->trace); }

    public function __toString(): string
    {
        $chain = [];
        for ($e = $this; $e !== null; $e = $e->getPrevious()) {
            $chain[] = $e;
        }
        $out = '';
        foreach (array_reverse($chain) as $i => $e) {
            $out .= ($i > 0 ? "\n\nNext " : '') . get_class($e) . ($e->getMessage() === '' ? '' : ': ' . $e->getMessage())
                . ' in ' . $e->getFile() . ':' . $e->getLine() . "\nStack trace:\n" . $e->getTraceAsString();
        }
        return $out;
    }
}

class Error implements Throwable
{
    protected $message = '';
    private string $string = '';
    protected $code = 0;
    protected string $file = '';
    protected int $line = 0;
    private array $trace = [];
    private ?Throwable $previous = null;

    public function __construct($message = '', $code = 0, ?Throwable $previous = null)
    {
        $this->message = $message;
        $this->code = $code;
        $this->previous = $previous;
    }

    final public function getMessage(): string { return $this->message; }
    final public function getCode() { return $this->code; }
    final public function getPrevious(): ?Throwable { return $this->previous; }
    final public function getFile(): string { return $this->file; }
    final public function getLine(): int { return $this->line; }
    final public function getTrace(): array { return $this->trace; }
    final public function getTraceAsString(): string { return __phpx_trace_string($this->trace); }

    public function __toString(): string
    {
        $chain = [];
        for ($e = $this; $e !== null; $e = $e->getPrevious()) {
            $chain[] = $e;
        }
        $out = '';
        foreach (array_reverse($chain) as $i => $e) {
            $out .= ($i > 0 ? "\n\nNext " : '') . get_class($e) . ($e->getMessage() === '' ? '' : ': ' . $e->getMessage())
                . ' in ' . $e->getFile() . ':' . $e->getLine() . "\nStack trace:\n" . $e->getTraceAsString();
        }
        return $out;
    }
}

class ErrorException extends Exception
{
    protected int $severity = E_ERROR;

    public function __construct(string $message = '', int $code = 0, int $severity = E_ERROR, ?string $filename = null, ?int $line = null, ?Throwable $previous = null)
    {
        parent::__construct($message, $code, $previous);
        $this->severity = $severity;
        if ($filename !== null) {
            $this->file = $filename;
        }
        if ($line !== null) {
            $this->line = $line;
        }
    }

    final public function getSeverity(): int { return $this->severity; }
}
class TypeError extends Error {}
class ValueError extends Error {}
class ArithmeticError extends Error {}
class DivisionByZeroError extends ArithmeticError {}
class CompileError extends Error {}
class ParseError extends CompileError {}
class ArgumentCountError extends TypeError {}
class UnhandledMatchError extends Error {}
class LogicException extends Exception {}
class BadFunctionCallException extends LogicException {}
class BadMethodCallException extends BadFunctionCallException {}
class DomainException extends LogicException {}
class InvalidArgumentException extends LogicException {}
class LengthException extends LogicException {}
class OutOfRangeException extends LogicException {}
class RuntimeException extends Exception {}
class OutOfBoundsException extends RuntimeException {}
class OverflowException extends RuntimeException {}
class RangeException extends RuntimeException {}
class UnderflowException extends RuntimeException {}
class UnexpectedValueException extends RuntimeException {}
class JsonException extends Exception {}

// phpx async exceptions.
final class TimeoutException extends RuntimeException {}
final class CancelledException extends RuntimeException {}
final class AggregateException extends RuntimeException
{
    public array $errors = [];

    public function getErrors(): array { return $this->errors; }
}

final class HashContext { private function __construct() {} }
class __PHP_Incomplete_Class {}

// ---- ext/fileinfo (natives in crates/interp/src/fileinfo.rs) ------------------------------

class finfo
{
    public function __construct(int $flags = FILEINFO_NONE, ?string $magic_database = null) {}
    public function file(string $filename, int $flags = FILEINFO_NONE, $context = null): string|false {}
    public function buffer(string $string, int $flags = FILEINFO_NONE, $context = null): string|false {}
    public function set_flags(int $flags): true {}
}
