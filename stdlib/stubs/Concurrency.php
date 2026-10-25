<?php

/**
 * IDE stub. Same API as Laravel's Concurrency facade, but tasks run as green
 * threads in the same process: no forking, no closure serialization.
 */
final class Concurrency
{
    /**
     * Run closures concurrently, wait for all, keep keys.
     * The first exception cancels the others and is rethrown.
     *
     * @template K of array-key
     * @template V
     * @param array<K, callable(): V> $tasks
     * @return array<K, V>
     * @throws Throwable
     */
    public static function run(array $tasks): array {}

    /**
     * Fire and forget after the response is sent. Same as defer() per task.
     *
     * @param array<array-key, callable(): mixed> $tasks
     */
    public static function defer(array $tasks): void {}
}
