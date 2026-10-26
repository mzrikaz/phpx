<?php
namespace App {
    #[\AllowDynamicProperties] class A {}
    class B extends A {}
    #[AllowDynamicProperties] class C {}   // App\AllowDynamicProperties: not the attribute
    class D {}
}
namespace {
    use AllowDynamicProperties as ADP;
    set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });
    #[ADP] class E {}
    foreach ([new App\A, new App\B, new App\C, new App\D, new E, new stdClass] as $o) {
        $o->x = 1;
        echo get_class($o), " ", $o->x, "\n";
    }
}
