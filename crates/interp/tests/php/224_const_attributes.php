<?php
// PHP 8.5: attributes on global constants (symfony/polyfill-uuid bootstrap85.php).
#[Deprecated(message: 'use B instead')]
const A = 4;

#[Foo]
#[Bar(1, 2)]
const B = 1;

echo A, B, "\n";
