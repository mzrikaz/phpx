<?php
set_error_handler(function ($n, $s) { echo "$n: $s\n"; return true; });
class A { #[\Deprecated("use B", "1.2")] const X = 1; #[Deprecated] const Y = 2; const Z = 3; }
class B extends A {}
echo A::X, B::X, A::Y, A::Z, "\n";
