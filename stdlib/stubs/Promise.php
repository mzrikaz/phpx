<?php

/**
 * IDE stub. Implemented natively by the phpx runtime (crates/runtime/src/promise.rs).
 *
 * A Promise is a handle to work that is already running in its own green
 * thread. Calling wait() parks only the current request, never the OS thread.
 *
 * @template T
 */
final class Promise
{
    private function __construct() {}

    /**
     * Park until settled. Returns the value or rethrows the exception.
     *
     * @return T
     * @throws Throwable
     */
    public function wait(): mixed {}

    /**
     * @template U
     * @param callable(T): U $onFulfilled
     * @return Promise<U>
     */
    public function then(callable $onFulfilled): Promise {}

    /**
     * @param callable(Throwable): T $onRejected
     * @return Promise<T>
     */
    public function catch(callable $onRejected): Promise {}

    /**
     * @param callable(): void $onSettled
     * @return Promise<T>
     */
    public function finally(callable $onSettled): Promise {}

    /**
     * Reject with TimeoutException after $seconds and cancel the work.
     *
     * @return Promise<T>
     */
    public function timeout(float $seconds): Promise {}

    /** Stop the underlying task at its next wait point. */
    public function cancel(): void {}

    public function isSettled(): bool {}

    /**
     * All fulfill, or reject with the first failure (the rest are cancelled).
     *
     * @template V
     * @param array<array-key, Promise<V>> $promises
     * @return Promise<array<array-key, V>>
     */
    public static function all(array $promises): Promise {}

    /**
     * Never rejects.
     *
     * @template V
     * @param array<array-key, Promise<V>> $promises
     * @return Promise<array<array-key, array{state: 'fulfilled', value: V}|array{state: 'rejected', reason: Throwable}>>
     */
    public static function allSettled(array $promises): Promise {}

    /**
     * First fulfillment wins; rejects with AggregateException if all fail.
     *
     * @template V
     * @param array<array-key, Promise<V>> $promises
     * @return Promise<V>
     */
    public static function any(array $promises): Promise {}

    /**
     * First to settle wins, success or failure.
     *
     * @template V
     * @param array<array-key, Promise<V>> $promises
     * @return Promise<V>
     */
    public static function race(array $promises): Promise {}
}

final class TimeoutException extends RuntimeException {}

final class CancelledException extends RuntimeException {}

final class AggregateException extends RuntimeException
{
    /** @return list<Throwable> */
    public function getErrors(): array {}
}
