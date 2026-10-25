<?php

/**
 * IDE stubs for global async helpers.
 */

/**
 * Start $fn in a new green thread right away and return its handle.
 *
 * @template T
 * @param callable(): T $fn
 * @return Promise<T>
 */
function async(callable $fn): Promise {}

/**
 * Run $fn after the response has been sent to the client.
 *
 * @param callable(): mixed $fn
 */
function defer(callable $fn): void {}
