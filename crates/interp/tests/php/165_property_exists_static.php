<?php
class P { public static $key = 'a'; protected static $p; private static $q; public $i; }
class Q extends P {}
var_dump(property_exists('P', 'key'), property_exists('Q', 'key'), property_exists('Q', 'p'), property_exists('Q', 'q'), property_exists(new Q, 'key'), property_exists('Q', 'i'), property_exists('Q', 'zz'));
